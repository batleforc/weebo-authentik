//! `AuthentikLocalInstance`: like an `AuthentikInstance`, accepted as is —
//! connectivity and token are checked lazily, by the gateway factory.

use std::sync::Arc;

use api::AuthentikLocalInstance;
use application::use_cases::ReconcileOutcome;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikLocalInstance>(client.clone());
    let health = ctx.health.clone();
    let results = super::controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikLocalInstance", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikLocalInstance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let namespace = obj.namespace().unwrap_or_default();
    let api: Api<AuthentikLocalInstance> = Api::namespaced(ctx.client.clone(), &namespace);

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(instance) => apply(&api, &instance, &ctx).await,
            FinalizerEvent::Cleanup(_instance) => Ok(Action::await_change()),
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikLocalInstance>,
    instance: &AuthentikLocalInstance,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let started = std::time::Instant::now();
    let outcome = ReconcileOutcome::Synced { authentik_id: None };
    super::record_reconcile("AuthentikLocalInstance", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, instance, outcome, "instance accepted")
        .await?;

    Ok(action)
}
