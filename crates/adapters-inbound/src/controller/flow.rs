use std::sync::Arc;

use api::AuthentikFlow;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_flow::reconcile_flow;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikFlow> = Api::all(client);
    let health = ctx.health.clone();
    let results = Controller::new(api, watcher::Config::default())
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikFlow", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikFlow>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikFlow> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(flow) => apply(&api, &flow, &ctx).await,
            FinalizerEvent::Cleanup(flow) => cleanup(&api, &flow, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(api: &Api<AuthentikFlow>, flow: &AuthentikFlow, ctx: &Ctx) -> Result<Action, Error> {
    let name = flow.name_any();
    // Re-fetched from the API server rather than trusted from the watch
    // cache, for the same reason spelled out in `controller/group.rs`: a
    // stale cached `authentik_id: None` right after a create would trigger
    // a spurious second create the gateway can never recover from.
    let current = api.get(&name).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_flow(flow, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikFlow", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, &current, outcome, "flow synced").await?;

    Ok(action)
}

async fn cleanup(
    _api: &Api<AuthentikFlow>,
    flow: &AuthentikFlow,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = flow.status.as_ref().and_then(|s| s.authentik_id.as_deref()) {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_flow(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, flow).await;
    Ok(Action::await_change())
}
