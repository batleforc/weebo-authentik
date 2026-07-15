mod telemetry;

use std::net::SocketAddr;
use std::sync::Arc;

use adapters_inbound::controller::{self, Ctx};
use adapters_inbound::webhook::{self, WebhookState};
use adapters_outbound::{AuthentikGatewayFactory, K8sSecretStore};
use application::ports::{GatewayFactory, SecretStore};
use kube::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;

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
    let secrets: Arc<dyn SecretStore> = Arc::new(K8sSecretStore::new(client.clone()));

    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets,
    });

    let webhook_state = WebhookState {
        client: client.clone(),
    };
    let app = webhook::router(webhook_state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8443));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(%addr, "starting weebo-authentik-operator");

    let (webhook_res, ..) = tokio::join!(
        async { axum::serve(listener, app).await },
        controller::instance::run(client.clone(), ctx.clone()),
        controller::group::run(client.clone(), ctx.clone()),
        controller::user::run(client.clone(), ctx.clone()),
        controller::outpost::run(client.clone(), ctx.clone()),
        controller::brand::run(client.clone(), ctx.clone()),
        controller::app::run(client.clone(), ctx.clone()),
        controller::access_policy::run(client.clone(), ctx.clone()),
        controller::namespace_policy::run(client, ctx),
    );
    webhook_res?;

    Ok(())
}
