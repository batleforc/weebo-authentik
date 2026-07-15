use std::sync::Arc;

use api::AuthentikUser;
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_user::reconcile_user;
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, FINALIZER, patch_ready_condition, patch_synced_status};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    #[error("finalizer error: {0}")]
    Finalizer(String),
    #[error("authentik gateway error: {0}")]
    Gateway(String),
}

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikUser> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikUser reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikUser>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikUser> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(user) => apply(&api, &user, &ctx).await,
            FinalizerEvent::Cleanup(user) => cleanup(&api, &user, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(api: &Api<AuthentikUser>, user: &AuthentikUser, ctx: &Ctx) -> Result<Action, Error> {
    let name = user.name_any();
    let authentik_id = user.status.as_ref().and_then(|s| s.authentik_id.clone());

    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_user(user, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };

    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => {
            patch_synced_status(api, &name, &id, "user synced").await?;
        }
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "user synced",
            )
            .await?;
        }
        ReconcileOutcome::Errored { reason, message } => {
            patch_ready_condition(api, &name, ConditionStatus::False, reason, message).await?;
        }
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

async fn cleanup(
    _api: &Api<AuthentikUser>,
    user: &AuthentikUser,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = user.status.as_ref().and_then(|s| s.authentik_id.as_deref()) {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_user(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    Ok(Action::await_change())
}

fn error_policy(_obj: Arc<AuthentikUser>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
