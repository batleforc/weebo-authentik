use std::sync::Arc;

use api::AuthentikInstance;
use application::use_cases::reconcile_instance::reconcile_instance;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikInstance> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikInstance reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikInstance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikInstance> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(instance) => apply(&api, &instance).await,
            FinalizerEvent::Cleanup(_instance) => Ok(Action::await_change()),
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikInstance>,
    instance: &AuthentikInstance,
) -> Result<Action, Error> {
    let name = instance.name_any();
    let started = std::time::Instant::now();
    let outcome = reconcile_instance(instance);
    super::record_reconcile("AuthentikInstance", started, &outcome);
    super::patch_reconcile_outcome(api, &name, outcome, "instance accepted").await?;

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}
