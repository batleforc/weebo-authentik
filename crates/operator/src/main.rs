use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx, Shutdown};
use adapters_inbound::webhook::{self, InstanceLookup, WebhookState};
use adapters_outbound::{
    AuthentikGatewayFactory, AuthentikInstances, AuthentikSecretStoreFactory, KubePolicySource,
};
use api::namespace_policy::NamespaceRule;
use api::{
    AuthentikApplication, AuthentikInstance, AuthentikLocalInstance, AuthentikNamespacePolicy,
};
use application::ports::{GatewayFactory, InstanceDirectory, PolicySource, SecretStoreFactory};
use kube::Client;
use kube::runtime::events::{Recorder, Reporter};
use weebo_kit_runtime::shared::{LISTED_TIMEOUT, SharedWatch};
use weebo_kit_runtime::{Health, WatchScope, leader, telemetry, webhook_server};

/// Default `LEASE_NAME`: what every release used before it became
/// configurable — keep it across upgrades, or old and new pods would each
/// hold "their" lease during the rollout.
const DEFAULT_LEASE_NAME: &str = "weebo-authentik-operator-leader";

/// The kit's runtime: `WEEBO_WORKER_THREADS` workers (4 by default), not
/// tokio's one per core (`weebo_kit_runtime::runtime`).
fn main() -> anyhow::Result<()> {
    weebo_kit_runtime::runtime::build()?.block_on(operator())
}

async fn operator() -> anyhow::Result<()> {
    let telemetry = telemetry::init("weebo-authentik-operator")?;
    let result = match std::env::args().nth(1).as_deref() {
        // What the image's smoke run calls: no cluster needed.
        Some("--version") => {
            println!("weebo-authentik-operator {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
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
    install_crypto_provider();
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

/// Both `ring` and `aws-lc-rs` are reachable transitively (`kube`'s
/// rustls-tls vs. `authentik-client`'s reqwest stack); rustls refuses to
/// pick a default automatically when more than one is linked.
fn install_crypto_provider() {
    // An error only says a provider is already installed: all this needs.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a rustls CryptoProvider was already installed");
    }
}

async fn run(telemetry: &telemetry::Telemetry) -> anyhow::Result<()> {
    install_crypto_provider();

    telemetry.serve_metrics().await?;
    // `weebo_authentik_leader`: 1 on the replica holding the lease.
    leader::register_leader_gauge("weebo-authentik-operator", "weebo_authentik");

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

    // One watch per kind, shared by every controller, by the policy
    // re-checks and by the instance and policy reads (every resolution reads
    // the instances: from memory, not a call per reconcile). Started before
    // campaigning for the lease: a standby keeps them warm, and the
    // controllers start from stores that already listed.
    let watches = controller::spawn_watches(&client, &scope, &health);
    let cluster_store = watches.get::<AuthentikInstance>().map(SharedWatch::store);
    let local_store = watches
        .get::<AuthentikLocalInstance>()
        .map(SharedWatch::store);
    let instances = Arc::new(
        AuthentikInstances::new(client.clone())
            .with_scope(
                scope.watched_namespace().map(String::from),
                scope.selector().cloned(),
                scope.cluster_kinds(),
            )
            .with_stores(cluster_store, local_store),
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
    let secret_stores = Arc::new(AuthentikSecretStoreFactory::new(
        client.clone(),
        instances.clone(),
    ));
    let secrets_factory: Arc<dyn SecretStoreFactory> = secret_stores.clone();
    let directory: Arc<dyn InstanceDirectory> = instances;

    // Every AuthentikNamespacePolicy, read by the webhook and re-checked by
    // the AuthentikApplication/AuthentikAccessPolicy reconcilers: from the
    // policies' shared watch (every policy, whatever its labels), none
    // without cluster-scoped kinds.
    let mut policy_source = KubePolicySource::new(client.clone());
    if let Some(shared) = watches.get_in::<AuthentikNamespacePolicy>(&scope.without_selector()) {
        policy_source = policy_source.with_store(shared.store());
    }
    let policies: Arc<dyn PolicySource<NamespaceRule>> = Arc::new(policy_source);

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
        watches,
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
                    vaults: secret_stores,
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
    // Authentik object concurrently. The replica campaigns once its shared
    // watches listed (their LISTs done before the lease, not in the
    // leader's first minute), or after LISTED_TIMEOUT: a watch that can't
    // list is reported by /livez, not waited for.
    let campaign = async {
        ctx.watches.wait_listed(LISTED_TIMEOUT).await;
        leader::acquire_leadership(&client, holder_id, DEFAULT_LEASE_NAME).await
    };
    let leadership = tokio::select! {
        leadership = campaign => leadership,
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
    if tokio::time::timeout(Duration::from_secs(30), webhook.stopped())
        .await
        .is_err()
    {
        tracing::warn!("webhook server still draining after 30s: exiting anyway");
    }
    Ok(())
}
