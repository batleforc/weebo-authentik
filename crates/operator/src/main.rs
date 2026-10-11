use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx, Shutdown};
use adapters_inbound::webhook::{self, WebhookState};
use adapters_outbound::{AuthentikGatewayFactory, AuthentikSecretStoreFactory, KubePolicySource};
use api::AuthentikInstance;
use api::namespace_policy::NamespaceRule;
use application::ports::{GatewayFactory, PolicySource, SecretStoreFactory};
use futures::StreamExt;
use kube::Client;
use kube::api::Api;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::{WatchStreamExt, reflector, watcher};
use weebo_kit_runtime::{Health, leader, telemetry, webhook_server};

/// Default `LEASE_NAME`: what every release used before it became
/// configurable — keep it across upgrades, or old and new pods would each
/// hold "their" lease during the rollout.
const DEFAULT_LEASE_NAME: &str = "weebo-authentik-operator-leader";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry = telemetry::init("weebo-authentik-operator")?;
    let result = run(&telemetry).await;
    if let Err(err) = &result {
        tracing::error!(error = %err, "operator failed");
    }
    telemetry.shutdown();
    result
}

async fn run(telemetry: &telemetry::Telemetry) -> anyhow::Result<()> {
    // Both `ring` and `aws-lc-rs` are reachable transitively (`kube`'s
    // rustls-tls vs. `authentik-client`'s reqwest stack); rustls refuses
    // to pick a default automatically when more than one is linked.
    let _ = rustls::crypto::ring::default_provider().install_default();

    telemetry.serve_metrics().await?;
    register_leader_gauge();

    let client = Client::try_default().await?;

    // The binary is PID 1 in its container: without a handler, SIGTERM
    // would be ignored and every rollout would wait for the SIGKILL.
    let shutdown = Shutdown::on_termination()?;
    let health = Arc::new(Health::default());

    // Both factories resolve `AuthentikInstance` CRs on every reconcile;
    // rather than each doing a per-call apiserver GET/LIST, they share a
    // single reflector-backed `Store` kept live by the watch spawned here.
    // Watch errors back off exponentially instead of spinning.
    let (instance_store, instance_writer) = reflector::store::<AuthentikInstance>();
    let instance_reflector = reflector::reflector(
        instance_writer,
        watcher(
            Api::<AuthentikInstance>::all(client.clone()),
            watcher::Config::default(),
        )
        .default_backoff(),
    );
    tokio::spawn({
        let health = health.clone();
        async move {
            instance_reflector
                .for_each(|event| {
                    match event {
                        Ok(_) => health.ok("AuthentikInstance reflector"),
                        Err(err) => {
                            tracing::warn!(error = %err, "AuthentikInstance reflector watch error");
                            health.failed("AuthentikInstance reflector", err.to_string());
                        }
                    }
                    futures::future::ready(())
                })
                .await;
        }
    });

    // Resolves an AuthentikGateway per AuthentikInstance CR (url from the
    // CR, token read fresh from its tokenSecretRef on every call — see
    // `application::ports::GatewayFactory`). CRDs with no `instanceRef`
    // field go through `default_gateway`, which requires exactly one
    // AuthentikInstance CR to exist.
    let gateway_factory: Arc<dyn GatewayFactory> = Arc::new(
        AuthentikGatewayFactory::with_instance_store(client.clone(), instance_store.clone()),
    );
    // Routes each secret target to Kubernetes or to the instance's Vault
    // (`spec.secretStore`), logging in to Vault only when a target needs it.
    let secrets_factory: Arc<dyn SecretStoreFactory> = Arc::new(
        AuthentikSecretStoreFactory::with_instance_store(client.clone(), instance_store),
    );

    // Every AuthentikNamespacePolicy, read by the webhook and re-checked by
    // the AuthentikApplication/AuthentikAccessPolicy reconcilers.
    let policies: Arc<dyn PolicySource<NamespaceRule>> =
        Arc::new(KubePolicySource::new(client.clone()));

    let holder_id = std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets_factory,
        policies: policies.clone(),
        recorder: Recorder::new(
            client.clone(),
            Reporter {
                controller: "weebo-authentik-operator".into(),
                instance: Some(holder_id.clone()),
            },
        ),
        shutdown: shutdown.clone(),
        health: health.clone(),
    });

    // The webhook serves on every replica, before and regardless of
    // leadership: with failurePolicy: Fail, a leader-only webhook would turn
    // every failover into a cluster-wide outage for AuthentikApplication/
    // AuthentikAccessPolicy writes. On shutdown it keeps answering while
    // endpoints drain.
    let webhook = webhook_server::spawn(
        webhook::router(
            WebhookState {
                policies: policies.clone(),
            },
            health.clone(),
        ),
        health,
        shutdown.clone(),
    )
    .await?;

    // The controllers, in contrast, must only run on one replica at a
    // time: two active reconcilers could both attempt-create the same
    // Authentik object concurrently.
    let leadership = tokio::select! {
        leadership = leader::acquire_leadership(&client, holder_id, DEFAULT_LEASE_NAME) => leadership,
        () = shutdown.wait() => {
            webhook.stopped().await;
            return Ok(());
        }
    };

    tracing::info!("starting controllers");
    tokio::join!(
        controller::instance::run(client.clone(), ctx.clone()),
        controller::group::run(client.clone(), ctx.clone()),
        controller::flow::run(client.clone(), ctx.clone()),
        controller::user::run(client.clone(), ctx.clone()),
        controller::outpost::run(client.clone(), ctx.clone()),
        controller::scope_mapping::run(client.clone(), ctx.clone()),
        controller::brand::run(client.clone(), ctx.clone()),
        controller::app::run(client.clone(), ctx.clone()),
        controller::access_policy::run(client.clone(), ctx.clone()),
        controller::namespace_policy::run(client, ctx),
    );
    // Every controller returns once the shutdown signal arrived and its
    // in-flight reconciles finished: hand the lease over right away
    // instead of making the next leader wait out the TTL.
    leadership.release().await;
    // Bounded: the drain started with the signal, long before.
    let _ = tokio::time::timeout(Duration::from_secs(30), webhook.stopped()).await;
    Ok(())
}

/// `weebo_authentik_leader`: 1 on the replica holding the lease.
fn register_leader_gauge() {
    // The callback is registered with the meter provider and lives as long
    // as it does: the handle itself can go.
    let _ = opentelemetry::global::meter("weebo-authentik-operator")
        .u64_observable_gauge("weebo_authentik_leader")
        .with_description("1 when this replica holds the leader lease")
        .with_callback(|observer| observer.observe(u64::from(leader::is_leader()), &[]))
        .build();
}
