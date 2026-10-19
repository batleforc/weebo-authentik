use std::sync::Arc;

use api::AuthentikGroup;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_group::reconcile_group;
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikGroup>(client.clone());
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikGroup", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikGroup>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikGroup> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(group) => apply(&api, &group, &ctx).await,
            FinalizerEvent::Cleanup(group) => {
                super::finalize(group.as_ref(), &ctx, cleanup(&api, &group, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikGroup>,
    group: &AuthentikGroup,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // Fresh from the API server, not the watch cache: see `super::current`.
    let current = super::current(api, group).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => {
            let outcome = reconcile_group(group, authentik_id.as_deref(), gateway.as_ref()).await;
            super::report_writes("AuthentikGroup", &ctx.recorder, &current, gateway.as_ref()).await;
            outcome
        }
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikGroup", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, &current, outcome, "group synced").await?;

    Ok(action)
}

async fn cleanup(
    _api: &Api<AuthentikGroup>,
    group: &AuthentikGroup,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = group
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;
        gateway
            .delete_group(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        super::count_writes("AuthentikGroup", gateway.as_ref());
    }
    super::announce_deleted(&ctx.recorder, group).await;
    Ok(Action::await_change())
}
