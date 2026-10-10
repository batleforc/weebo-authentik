use std::sync::Arc;

use api::AuthentikInstance;
use application::use_cases::reconcile_instance::reconcile_instance;
use kube::Client;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikInstance> = Api::all(client);
    let health = ctx.health.clone();
    let results = Controller::new(api, watcher::Config::default())
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikInstance", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikInstance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikInstance> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(instance) => apply(&api, &instance, &ctx).await,
            FinalizerEvent::Cleanup(_instance) => Ok(Action::await_change()),
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
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
