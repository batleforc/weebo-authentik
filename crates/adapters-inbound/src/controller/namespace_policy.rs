use std::sync::Arc;

use api::AuthentikNamespacePolicy;
use application::use_cases::reconcile_namespace_policy::reconcile_namespace_policy;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikNamespacePolicy> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikNamespacePolicy reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikNamespacePolicy>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikNamespacePolicy> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(policy) => apply(&api, &policy).await,
            FinalizerEvent::Cleanup(_policy) => Ok(Action::await_change()),
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikNamespacePolicy>,
    policy: &AuthentikNamespacePolicy,
) -> Result<Action, Error> {
    let name = policy.name_any();
    let started = std::time::Instant::now();
    let outcome = reconcile_namespace_policy(policy);
    super::record_reconcile("AuthentikNamespacePolicy", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &name, outcome, "policy accepted").await?;

    Ok(action)
}
