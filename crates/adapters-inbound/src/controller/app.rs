use std::sync::Arc;

use api::AuthentikApplication;
use api::application::ProviderKind;
use application::use_cases::errored_from_factory_error;
use application::use_cases::errored_from_secret_store_factory_error;
use application::use_cases::reconcile_application::reconcile_application;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

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
    let started = std::time::Instant::now();
    let authentik_id = app.status.as_ref().and_then(|s| s.authentik_id.clone());

    let outcome = match ctx
        .gateway_factory
        .gateway_for(&app.spec.instance_ref)
        .await
    {
        Ok(gateway) => match ctx
            .secrets_factory
            .secret_store_for(&app.spec.instance_ref)
            .await
        {
            Ok(secrets) => {
                reconcile_application(
                    app,
                    authentik_id.as_deref(),
                    gateway.as_ref(),
                    secrets.as_ref(),
                )
                .await
            }
            Err(e) => errored_from_secret_store_factory_error(e),
        },
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikApplication", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &name, outcome, "application synced").await?;

    Ok(action)
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
            let secrets = ctx
                .secrets_factory
                .secret_store_for(&app.spec.instance_ref)
                .await
                .map_err(|e| Error::SecretStore(e.to_string()))?;
            secrets
                .delete(&namespace, &name)
                .await
                .map_err(|e| Error::SecretStore(e.to_string()))?;
        }
    }
    Ok(Action::await_change())
}
