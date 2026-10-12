use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx, Shutdown};
use adapters_inbound::webhook::{self, InstanceLookup, WebhookState};
use adapters_outbound::{
    AuthentikGatewayFactory, AuthentikInstances, AuthentikSecretStoreFactory, KubePolicySource,
};
use api::namespace_policy::NamespaceRule;
use api::{AuthentikApplication, AuthentikInstance, AuthentikLocalInstance};
use application::ports::{GatewayFactory, InstanceDirectory, PolicySource, SecretStoreFactory};
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::reflector::Store;
use kube::runtime::{WatchStreamExt, reflector, watcher};
use kube::{Client, Resource};
use serde::de::DeserializeOwned;
use weebo_kit_runtime::{Health, WatchScope, leader, telemetry, webhook_server};

/// Default `LEASE_NAME`: what every release used before it became
/// configurable — keep it across upgrades, or old and new pods would each
/// hold "their" lease during the rollout.
const DEFAULT_LEASE_NAME: &str = "weebo-authentik-operator-leader";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry = telemetry::init("weebo-authentik-operator")?;
    let result = match std::env::args().nth(1).as_deref() {
        Some("migrate-instance-ref") => migrate_instance_ref().await,
        Some(other) => Err(anyhow::anyhow!("unknown command {other:?}")),
        None => run(&telemetry).await,
    };
    if let Err(err) = &result {
        tracing::error!(error = %err, "operator failed");
    }
    telemetry.shutdown();
    result
}

/// `migrate-instance-ref [--dry-run]`: rewrites the `AuthentikApplication`s
/// written when `spec.instanceRef` was a bare name (`prod` → `{name:
/// prod}`). Offline: operator stopped, new CRDs applied, then this, then
/// the operator.
async fn migrate_instance_ref() -> anyhow::Result<()> {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = Client::try_default().await?;
    let kinds = [kube::api::ApiResource::erase::<AuthentikApplication>(&())];
    let migration = weebo_kit_adapters::migrate::migrate_instance_refs(
        &client,
        &kinds,
        "weebo-authentik-operator",
        dry_run,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    let verb = if dry_run { "to migrate" } else { "migrated" };
    for id in &migration.migrated {
        println!("{verb}: {id}");
    }
    println!(
        "{} {verb}, {} unchanged",
        migration.migrated.len(),
        migration.unchanged
    );
    Ok(())
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

    // The whole cluster, or one namespace (with or without the
    // cluster-scoped kinds) and/or a label selector (the chart's
    // `watch.*`): an invalid value stops the start here, as does a kind
    // the scope can't watch.
    let scope = WatchScope::from_env()?;
    scope.ensure::<AuthentikLocalInstance>()?;
    scope.ensure::<AuthentikApplication>()?;
    if scope.cluster_kinds() {
        scope.ensure::<AuthentikInstance>()?;
    }
    tracing::info!(
        namespace = ?scope.watched_namespace(),
        selector = ?scope.selector(),
        cluster_kinds = scope.cluster_kinds(),
        "watch scope"
    );

    // Both factories and every instanceRef resolution read the instances;
    // rather than a per-call apiserver GET/LIST, they share one
    // reflector-backed `Store` per kind (no AuthentikInstance without
    // cluster-scoped kinds).
    let cluster_store = scope.cluster_kinds().then(|| {
        spawn_reflector(
            scope.api::<AuthentikInstance>(client.clone()),
            &scope,
            health.clone(),
            "AuthentikInstance reflector",
        )
    });
    let local_store = spawn_reflector(
        scope.api::<AuthentikLocalInstance>(client.clone()),
        &scope,
        health.clone(),
        "AuthentikLocalInstance reflector",
    );
    let instances = Arc::new(
        AuthentikInstances::new(client.clone())
            .with_scope(
                scope.watched_namespace().map(String::from),
                scope.selector().cloned(),
                scope.cluster_kinds(),
            )
            .with_stores(cluster_store, Some(local_store)),
    );

    // Resolves an AuthentikGateway per resolved instance (url from the
    // CR, token read fresh from its tokenSecretRef on every call — see
    // `application::ports::GatewayFactory`). CRDs with no `instanceRef`
    // field go through `default_gateway`, which requires exactly one
    // AuthentikInstance CR to exist.
    let gateway_factory: Arc<dyn GatewayFactory> = Arc::new(AuthentikGatewayFactory::new(
        client.clone(),
        instances.clone(),
    ));
    // Routes each secret target to Kubernetes or to the instance's Vault
    // (`spec.secretStore`), logging in to Vault only when a target needs it.
    let secrets_factory: Arc<dyn SecretStoreFactory> = Arc::new(AuthentikSecretStoreFactory::new(
        client.clone(),
        instances.clone(),
    ));
    let directory: Arc<dyn InstanceDirectory> = instances;

    // Every AuthentikNamespacePolicy, read by the webhook and re-checked by
    // the AuthentikApplication/AuthentikAccessPolicy reconcilers.
    let policies: Arc<dyn PolicySource<NamespaceRule>> =
        Arc::new(KubePolicySource::new(client.clone()));

    let holder_id = std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets_factory,
        instances: directory.clone(),
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
        scope: scope.clone(),
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
                instances: Some(InstanceLookup {
                    client: client.clone(),
                    directory,
                }),
            },
            health.clone(),
            scope.clone(),
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
    // Without cluster-scoped kinds (full namespaced), only the namespaced
    // ones: the local instances, applications and their access policies.
    let cluster_kinds = async {
        if ctx.scope.cluster_kinds() {
            tokio::join!(
                controller::instance::run(client.clone(), ctx.clone()),
                controller::group::run(client.clone(), ctx.clone()),
                controller::flow::run(client.clone(), ctx.clone()),
                controller::user::run(client.clone(), ctx.clone()),
                controller::outpost::run(client.clone(), ctx.clone()),
                controller::scope_mapping::run(client.clone(), ctx.clone()),
                controller::brand::run(client.clone(), ctx.clone()),
                controller::namespace_policy::run(client.clone(), ctx.clone()),
            );
        }
    };
    tokio::join!(
        cluster_kinds,
        controller::local_instance::run(client.clone(), ctx.clone()),
        controller::app::run(client.clone(), ctx.clone()),
        controller::access_policy::run(client.clone(), ctx.clone()),
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

/// A reflector on `api`, its watch errors reported to `health`: the store
/// serves every read from memory, and watch errors back off exponentially
/// instead of spinning.
fn spawn_reflector<K>(
    api: Api<K>,
    scope: &WatchScope,
    health: Arc<Health>,
    name: &'static str,
) -> Store<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    let (store, writer) = reflector::store::<K>();
    let stream = reflector::reflector(
        writer,
        watcher(api, scope.watcher_config()).default_backoff(),
    );
    tokio::spawn(async move {
        stream
            .for_each(|event| {
                match event {
                    Ok(_) => health.ok(name),
                    Err(err) => {
                        tracing::warn!(error = %err, "{name} watch error");
                        health.failed(name, err.to_string());
                    }
                }
                futures::future::ready(())
            })
            .await;
    });
    store
}
