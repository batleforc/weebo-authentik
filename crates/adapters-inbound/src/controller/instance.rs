use std::sync::Arc;

use api::AuthentikInstance;
use application::ports::BoundInstance;
use application::use_cases::reconcile_instance::reconcile_instance;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikInstance>(client.clone());
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikInstance", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikInstance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikInstance> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(instance) => apply(&api, &instance, &ctx).await,
            FinalizerEvent::Cleanup(instance) => {
                super::finalize(instance.as_ref(), &ctx, cleanup(&api, &instance, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

/// Held while CRs are still bound to the instance: their own finalizers
/// need it to clean up, and would otherwise stay stuck in Terminating.
async fn cleanup(
    api: &Api<AuthentikInstance>,
    instance: &AuthentikInstance,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // The default of the cluster-scoped kinds when it is the only one.
    let default = api.list(&Default::default()).await?.items.len() == 1;
    super::release_instance(ctx, &BoundInstance::cluster(instance.name_any()), default).await
}

async fn apply(
    api: &Api<AuthentikInstance>,
    instance: &AuthentikInstance,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let started = std::time::Instant::now();
    let outcome = reconcile_instance(instance);
    super::record_reconcile("AuthentikInstance", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, instance, outcome, "instance accepted")
        .await?;

    Ok(action)
}
