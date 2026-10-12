use std::sync::Arc;

use api::AuthentikOutpost;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_outpost::reconcile_outpost;
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikOutpost>(client.clone());
    let health = ctx.health.clone();
    let results = super::controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikOutpost", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikOutpost>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikOutpost> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(outpost) => apply(&api, &outpost, &ctx).await,
            FinalizerEvent::Cleanup(outpost) => {
                super::finalize(outpost.as_ref(), &ctx, cleanup(&api, &outpost, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikOutpost>,
    outpost: &AuthentikOutpost,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let authentik_id = outpost.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_outpost(outpost, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikOutpost", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, outpost, outcome, "outpost synced").await?;

    Ok(action)
}

async fn cleanup(
    _api: &Api<AuthentikOutpost>,
    outpost: &AuthentikOutpost,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = outpost
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_outpost(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, outpost).await;
    Ok(Action::await_change())
}
