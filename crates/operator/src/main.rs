mod telemetry;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx};
use adapters_inbound::webhook::{self, WebhookState};
use adapters_outbound::{AuthentikGatewayFactory, AuthentikSecretStoreFactory};
use application::ports::{GatewayFactory, SecretStoreFactory};
use axum_server::tls_rustls::RustlsConfig;
use kube::Client;
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};

/// How often the webhook's TLS cert/key are re-read from disk, so
/// cert-manager's periodic rotation is picked up without a pod restart.
/// No filesystem watcher is used — a Secret-volume update already lands
/// with its own propagation delay, and a fixed poll is simpler/more
/// robust than reacting to the intermediate states of an atomic-rename
/// volume update.
const TLS_RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// Same 3:1 renew:ttl ratio as `kube-leader-election`'s own example —
/// enough headroom to survive a couple of missed renewals from GC pauses
/// or transient API server hiccups without losing leadership spuriously.
const LEASE_TTL: Duration = Duration::from_secs(15);
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;

    // Both `ring` and `aws-lc-rs` are reachable transitively (`kube`'s
    // rustls-tls vs. `authentik-client`'s reqwest stack); rustls refuses
    // to pick a default automatically when more than one is linked. See
    // `testkit::envtest::EnvTestCluster::start` for the same fix applied
    // to the test harness.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let client = Client::try_default().await?;

    // Resolves an AuthentikGateway per AuthentikInstance CR (url/token
    // come from the CR's tokenSecretRef, read fresh on every call — see
    // `application::ports::GatewayFactory`). CRDs with no `instanceRef`
    // field go through `default_gateway`, which requires exactly one
    // AuthentikInstance CR to exist; true multi-instance cohabitation for
    // those CRDs remains the explicitly-deferred item from
    // `.prompt/plan.md`.
    let gateway_factory: Arc<dyn GatewayFactory> =
        Arc::new(AuthentikGatewayFactory::new(client.clone()));
    // Resolves K8sSecretStore/VaultSecretStore per AuthentikInstance CR
    // (`spec.secretStore`, defaulting to Kubernetes) — see
    // `application::ports::SecretStoreFactory`.
    let secrets_factory: Arc<dyn SecretStoreFactory> =
        Arc::new(AuthentikSecretStoreFactory::new(client.clone()));

    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets_factory,
    });

    let webhook_state = WebhookState {
        client: client.clone(),
    };
    let app = webhook::router(webhook_state);
    let port: u16 = std::env::var("WEBHOOK_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8443);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // A `ValidatingWebhookConfiguration` only ever calls over HTTPS —
    // the cert/key are cert-manager-issued, mounted from a
    // `kubernetes.io/tls` Secret (see `.prompt/plan.md` decision 1).
    let tls_cert_file = std::env::var("WEBHOOK_TLS_CERT_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.crt".to_string());
    let tls_key_file = std::env::var("WEBHOOK_TLS_KEY_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.key".to_string());
    let tls_config = RustlsConfig::from_pem_file(&tls_cert_file, &tls_key_file).await?;

    tokio::spawn({
        let tls_config = tls_config.clone();
        async move {
            let mut interval = tokio::time::interval(TLS_RELOAD_INTERVAL);
            interval.tick().await; // first tick fires immediately; cert is already fresh from startup
            loop {
                interval.tick().await;
                if let Err(err) = tls_config
                    .reload_from_pem_file(&tls_cert_file, &tls_key_file)
                    .await
                {
                    tracing::error!(error = %err, "failed to reload webhook TLS cert, keeping previous one");
                }
            }
        }
    });

    tracing::info!(%addr, "starting weebo-authentik-operator webhook");

    // The webhook must serve every replica, always — `failurePolicy:
    // Fail` on the ValidatingWebhookConfiguration means a webhook that's
    // only up on the leader would turn routine leader failover into a
    // cluster-wide outage for AuthentikApplication/AuthentikAccessPolicy
    // writes. It is therefore spawned independently of leader election,
    // never gated behind it.
    let webhook_handle = tokio::spawn(async move {
        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
    });

    // The controllers, in contrast, must only run on one replica at a
    // time — see `.prompt/plan.md`, "HA du controller manager: leader
    // election standard": two active reconcilers could both
    // attempt-create the same Authentik object concurrently.
    let pod_namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let holder_id = std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let lease_name = std::env::var("LEASE_NAME")
        .unwrap_or_else(|_| "weebo-authentik-operator-leader".to_string());
    let lease = Arc::new(LeaseLock::new(
        client.clone(),
        &pod_namespace,
        LeaseLockParams {
            holder_id,
            lease_name,
            lease_ttl: LEASE_TTL,
        },
    ));

    tracing::info!("waiting to acquire leader lease before starting reconcilers");
    loop {
        match lease.try_acquire_or_renew().await {
            Ok(LeaseLockResult::Acquired(_)) => break,
            Ok(LeaseLockResult::NotAcquired(_)) => {}
            Err(err) => {
                tracing::error!(error = %err, "leader lease acquisition attempt failed, retrying")
            }
        }
        tokio::time::sleep(LEASE_RENEW_INTERVAL).await;
    }
    tracing::info!("acquired leader lease, starting reconcilers");

    tokio::spawn({
        let lease = lease.clone();
        async move {
            let mut interval = tokio::time::interval(LEASE_RENEW_INTERVAL);
            interval.tick().await; // first tick fires immediately; lease was just acquired above
            loop {
                interval.tick().await;
                match lease.try_acquire_or_renew().await {
                    Ok(LeaseLockResult::Acquired(_)) => {}
                    Ok(LeaseLockResult::NotAcquired(_)) => {
                        tracing::error!(
                            "lost leader lease, exiting so another replica can take over"
                        );
                        std::process::exit(1);
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "failed to renew leader lease, exiting so another replica can take over");
                        std::process::exit(1);
                    }
                }
            }
        }
    });

    tokio::join!(
        controller::instance::run(client.clone(), ctx.clone()),
        controller::group::run(client.clone(), ctx.clone()),
        controller::user::run(client.clone(), ctx.clone()),
        controller::outpost::run(client.clone(), ctx.clone()),
        controller::brand::run(client.clone(), ctx.clone()),
        controller::app::run(client.clone(), ctx.clone()),
        controller::access_policy::run(client.clone(), ctx.clone()),
        controller::namespace_policy::run(client, ctx),
    );

    webhook_handle.await??;

    Ok(())
}
