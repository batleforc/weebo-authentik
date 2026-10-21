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
    let results = super::on_policy_change(ctx.watches.controller(api, &ctx.scope), &client, &ctx)
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
        authentik_id: None,
        provider_id: None,
    };
    let ApplicationReconciled {
        outcome,
        authentik_id: written_id,
        provider_id,
    } = match (refusal, &bound) {
        // Re-checked here too: a policy may have changed since admission.
        (Some(refused), _) => unrecorded(refused),
        (None, Err(e)) => unrecorded(errored_from_instance_error(e.clone())),
        (None, Ok(bound)) => match ctx.gateway_factory.gateway_for(bound).await {
            Ok(gateway) => match ctx.secrets_factory.store_for(bound).await {
                Ok(secrets) => {
                    let reconciled = reconcile_application_recording(
                        app,
                        authentik_id.as_deref(),
                        gateway.as_ref(),
                        secrets.as_ref(),
                    )
                    .await;
                    super::report_writes(
                        "AuthentikApplication",
                        &ctx.recorder,
                        app,
                        gateway.as_ref(),
                    )
                    .await;
                    reconciled
                }
                Err(e) => unrecorded(errored_from_secret_store_error(e)),
            },
            Err(e) => unrecorded(errored_from_factory_error(e)),
        },
    };
    // What exists in Authentik is recorded even when a later step failed:
    // the next attempt patches it instead of colliding with it. Nothing
    // written leaves the recorded ids as they were.
    let mut fields = serde_json::Map::new();
    if let Some(provider_id) = provider_id {
        fields.insert("providerId".into(), provider_id.into());
    }
    if let Some(id) = written_id {
        fields.insert("authentikId".into(), id.into());
    }
    super::record_reconcile("AuthentikApplication", started, &outcome);
    let action = super::requeue_after(app, &outcome);
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
    let authentik_id = app.status.as_ref().and_then(|s| s.authentik_id.as_deref());
    // Recorded alone when the application itself was never created.
    let recorded_provider = recorded_provider_id(app);
    if authentik_id.is_some() || recorded_provider.is_some() {
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
        // The recorded provider, whatever its kind: the spec may name
        // another kind since it was created.
        let delete_recorded_provider = async {
            if let Some(provider_id) = recorded_provider {
                gateway
                    .delete_any_provider(provider_id)
                    .await
                    .map_err(|e| Error::Gateway(e.to_string()))?;
            }
            Ok::<(), Error>(())
        };
        match authentik_id {
            // Read the provider relationship before deleting the
            // application.
            Some(id) => match gateway.get_application(id).await {
                Ok(remote) => {
                    match (
                        remote.provider_id,
                        remote
                            .provider_meta_model_name
                            .as_deref()
                            .and_then(|name| match name {
                                "authentik_providers_oauth2.oauth2provider" => {
                                    Some(ProviderKind::Oauth2)
                                }
                                "authentik_providers_proxy.proxyprovider" => {
                                    Some(ProviderKind::Proxy)
                                }
                                _ => None,
                            }),
                    ) {
                        (Some(provider_id), Some(provider_kind)) => {
                            gateway
                                .delete_provider(provider_id, provider_kind)
                                .await
                                .map_err(|e| Error::Gateway(e.to_string()))?;
                            // Repointed out of band: the one recorded is
                            // left behind by the application.
                            if recorded_provider != Some(provider_id) {
                                delete_recorded_provider.await?;
                            }
                        }
                        // An application pointing at no provider, or
                        // repointed out of band to a provider of a kind
                        // this operator never creates (left alone, it isn't
                        // this CR's): the one recorded, if any.
                        _ => delete_recorded_provider.await?,
                    }
                    gateway
                        .delete_application(id)
                        .await
                        .map_err(|e| Error::Gateway(e.to_string()))?;
                }
                // Already deleted out of band: the provider it left behind.
                Err(GatewayError::Gone(_)) => delete_recorded_provider.await?,
                Err(e) => return Err(Error::Gateway(e.to_string())),
            },
            None => delete_recorded_provider.await?,
        }
        super::count_writes("AuthentikApplication", gateway.as_ref());

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
