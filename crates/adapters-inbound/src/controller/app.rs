use std::sync::Arc;

use api::AuthentikApplication;
use api::application::ProviderKind;
use application::ports::GatewayError;
use application::use_cases::reconcile_application::{
    ApplicationReconciled, delete_credentials, reconcile_application_recording,
    recorded_provider_id,
};
use application::use_cases::{
    errored_from_factory_error, errored_from_instance_error, errored_from_secret_store_error,
};
use domain::allow_list::ResourceKind;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikApplication>(client.clone());
    let health = ctx.health.clone();
    let results = super::on_policy_change(super::controller(api, &ctx.scope), client, &ctx.scope)
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
    let current = super::current(api, app).await?;
    let app = &current;
    let authentik_id = app.status.as_ref().and_then(|s| s.authentik_id.clone());

    let namespace = app.namespace().unwrap_or_default();
    let bound = super::application_instance(ctx, app).await;
    let refusal = super::policy_refusal(
        ctx,
        &namespace,
        ResourceKind::AuthentikApplication,
        bound.as_ref().ok(),
    )
    .await?;
    let unrecorded = |outcome| ApplicationReconciled {
        outcome,
        provider_id: None,
    };
    let ApplicationReconciled {
        outcome,
        provider_id,
    } = match (refusal, &bound) {
        // Re-checked here too: a policy may have changed since admission.
        (Some(refused), _) => unrecorded(refused),
        (None, Err(e)) => unrecorded(errored_from_instance_error(e.clone())),
        (None, Ok(bound)) => match ctx.gateway_factory.gateway_for(bound).await {
            Ok(gateway) => match ctx.secrets_factory.store_for(bound).await {
                Ok(secrets) => {
                    reconcile_application_recording(
                        app,
                        authentik_id.as_deref(),
                        gateway.as_ref(),
                        secrets.as_ref(),
                    )
                    .await
                }
                Err(e) => unrecorded(errored_from_secret_store_error(e)),
            },
            Err(e) => unrecorded(errored_from_factory_error(e)),
        },
    };
    // Only a synced application records its provider; a failure leaves the
    // recorded one as it was.
    let mut fields = serde_json::Map::new();
    if let Some(provider_id) = provider_id {
        fields.insert("providerId".into(), provider_id.into());
    }
    super::record_reconcile("AuthentikApplication", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome_with(
        api,
        &ctx.recorder,
        app,
        outcome,
        "application synced",
        bound.as_ref().ok(),
        fields,
    )
    .await?;

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
        // The instance it was bound to, not whatever its ref designates now.
        let bound = match app.status.as_ref().and_then(|s| s.instance.clone()) {
            Some(bound) => bound,
            None => super::application_instance(ctx, app)
                .await
                .map_err(|e| Error::cleanup(errored_from_instance_error(e)))?,
        };
        let gateway = ctx
            .gateway_factory
            .gateway_for(&bound)
            .await
            .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;
        // Read the provider relationship before deleting the application.
        // Already deleted out of band: the provider recorded in the status
        // (of the spec's kind) is deleted, then the credentials.
        match gateway.get_application(id).await {
            Ok(remote) => {
                if let (Some(provider_id), Some(provider_kind)) = (
                    remote.provider_id,
                    remote
                        .provider_meta_model_name
                        .as_deref()
                        .and_then(|name| match name {
                            "authentik_providers_oauth2.oauth2provider" => {
                                Some(ProviderKind::Oauth2)
                            }
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
            }
            Err(GatewayError::Gone(_)) => {
                if let Some(provider_id) = recorded_provider_id(app)
                    && matches!(
                        app.spec.provider.kind,
                        ProviderKind::Oauth2 | ProviderKind::Proxy
                    )
                {
                    gateway
                        .delete_provider(provider_id, app.spec.provider.kind.clone())
                        .await
                        .map_err(|e| Error::Gateway(e.to_string()))?;
                }
            }
            Err(e) => return Err(Error::Gateway(e.to_string())),
        }

        if matches!(app.spec.provider.kind, ProviderKind::Oauth2) {
            let secrets = ctx
                .secrets_factory
                .store_for(&bound)
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
