use std::sync::Arc;

use api::AuthentikApplication;
use api::application::ProviderKind;
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_application::reconcile_application;
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
    #[error("secret store error: {0}")]
    SecretStore(String),
}

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikApplication> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikApplication reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikApplication>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let ns = obj.namespace().unwrap_or_default();
    let api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), &ns);

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(app) => apply(&api, &app, &ctx).await,
            FinalizerEvent::Cleanup(app) => cleanup(&api, &app, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikApplication>,
    app: &AuthentikApplication,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = app.name_any();
    let authentik_id = app.status.as_ref().and_then(|s| s.authentik_id.clone());

    let outcome = match ctx
        .gateway_factory
        .gateway_for(&app.spec.instance_ref)
        .await
    {
        Ok(gateway) => {
            reconcile_application(
                app,
                authentik_id.as_deref(),
                gateway.as_ref(),
                ctx.secrets.as_ref(),
            )
            .await
        }
        Err(e) => errored_from_factory_error(e),
    };

    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => {
            patch_synced_status(api, &name, &id, "application synced").await?;
        }
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "application synced",
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
    _api: &Api<AuthentikApplication>,
    app: &AuthentikApplication,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // The finalizer isn't removed until this returns Ok, so a failed
    // cleanup keeps retrying — never silently drops the finalizer.
    if let Some(id) = app.status.as_ref().and_then(|s| s.authentik_id.as_deref()) {
        let gateway = ctx
            .gateway_factory
            .gateway_for(&app.spec.instance_ref)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_application(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;

        if matches!(app.spec.provider.kind, ProviderKind::Oauth2) {
            let namespace = app.metadata.namespace.clone().unwrap_or_default();
            let name = app.name_any();
            ctx.secrets
                .delete(&namespace, &name)
                .await
                .map_err(|e| Error::SecretStore(e.to_string()))?;
        }
    }
    Ok(Action::await_change())
}

fn error_policy(_obj: Arc<AuthentikApplication>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
