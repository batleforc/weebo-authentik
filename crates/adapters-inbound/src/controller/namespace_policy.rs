use std::sync::Arc;

use api::AuthentikNamespacePolicy;
use application::use_cases::ReconcileOutcome;
use application::use_cases::reconcile_namespace_policy::reconcile_namespace_policy;
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, FINALIZER, patch_ready_condition};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    #[error("finalizer error: {0}")]
    Finalizer(String),
}

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

    match reconcile_namespace_policy(policy) {
        ReconcileOutcome::Synced { .. } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "policy accepted",
            )
            .await?;
        }
        ReconcileOutcome::Errored { reason, message } => {
            patch_ready_condition(api, &name, ConditionStatus::False, reason, message).await?;
        }
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

fn error_policy(_obj: Arc<AuthentikNamespacePolicy>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
