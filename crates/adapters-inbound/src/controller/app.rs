use std::sync::Arc;

use api::AuthentikApplication;
use api::application::ProviderKind;
use application::use_cases::errored_from_factory_error;
use application::use_cases::errored_from_secret_store_error;
use application::use_cases::reconcile_application::{delete_credentials, reconcile_application};
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikApplication> = Api::all(client);
    let health = ctx.health.clone();
    let results = super::controller(api)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikApplication", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikApplication>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let ns = obj.namespace().unwrap_or_default();
    let api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), &ns);

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(app) => apply(&api, &app, &ctx).await,
            FinalizerEvent::Cleanup(app) => {
                super::finalize(app.as_ref(), &ctx, cleanup(&api, &app, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikApplication>,
    app: &AuthentikApplication,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let started = std::time::Instant::now();
    let authentik_id = app.status.as_ref().and_then(|s| s.authentik_id.clone());

    let outcome = match ctx
        .gateway_factory
        .gateway_for(&app.spec.instance_ref)
        .await
    {
        Ok(gateway) => match ctx
            .secrets_factory
            .store_for(Some(&app.spec.instance_ref))
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
            Err(e) => errored_from_secret_store_error(e),
        },
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikApplication", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, app, outcome, "application synced").await?;

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
        // Read the provider relationship before deleting the application.
        // The status stores only the application's slug.
        let remote = gateway
            .get_application(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        if let (Some(provider_id), Some(provider_kind)) = (
            remote.provider_id,
            remote
                .provider_meta_model_name
                .as_deref()
                .and_then(|name| match name {
                    "authentik_providers_oauth2.oauth2provider" => Some(ProviderKind::Oauth2),
                    "authentik_providers_proxy.proxyprovider" => Some(ProviderKind::Proxy),
                    _ => None,
                }),
        ) {
            gateway
                .delete_provider(provider_id, provider_kind)
                .await
                .map_err(|e| Error::Gateway(e.to_string()))?;
        }
        gateway
            .delete_application(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;

        if matches!(app.spec.provider.kind, ProviderKind::Oauth2) {
            let secrets = ctx
                .secrets_factory
                .store_for(Some(&app.spec.instance_ref))
                .await
                .map_err(|e| Error::SecretStore(e.to_string()))?;
            delete_credentials(app, secrets.as_ref())
                .await
                .map_err(|e| Error::SecretStore(e.to_string()))?;
        }
    }
    super::announce_deleted(&ctx.recorder, app).await;
    Ok(Action::await_change())
}
