use std::sync::Arc;

use api::{AuthentikAccessPolicy, AuthentikApplication};
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_access_policy::{
    application_ref_not_found, reconcile_access_policy,
};
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikAccessPolicy> = Api::all(client);
    let health = ctx.health.clone();
    let results = super::controller(api)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikAccessPolicy", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikAccessPolicy>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let ns = obj.namespace().unwrap_or_default();
    let api: Api<AuthentikAccessPolicy> = Api::namespaced(ctx.client.clone(), &ns);

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(policy) => apply(&api, &policy, &ctx, &ns).await,
            FinalizerEvent::Cleanup(policy) => {
                super::finalize(policy.as_ref(), &ctx, cleanup(&api, &policy, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikAccessPolicy>,
    policy: &AuthentikAccessPolicy,
    ctx: &Ctx,
    namespace: &str,
) -> Result<Action, Error> {
    let started = std::time::Instant::now();
    let authentik_id = policy.status.as_ref().and_then(|s| s.authentik_id.clone());

    // Same-namespace lookup, never a cluster-wide client — this is the
    // enforcement mechanism from `.prompt/plan.md`, "Confinement au
    // namespace, deux couches": a same-named AuthentikApplication in a
    // different namespace is invisible here by construction.
    let app_api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), namespace);
    let resolved = app_api.get_opt(&policy.spec.application_ref).await?;

    let outcome = match &resolved {
        Some(application) => {
            match ctx
                .gateway_factory
                .gateway_for(&application.spec.instance_ref)
                .await
            {
                Ok(gateway) => {
                    reconcile_access_policy(
                        policy,
                        application,
                        authentik_id.as_deref(),
                        gateway.as_ref(),
                    )
                    .await
                }
                Err(e) => errored_from_factory_error(e),
            }
        }
        None => application_ref_not_found(policy),
    };
    super::record_reconcile("AuthentikAccessPolicy", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, policy, outcome, "access policy synced")
        .await?;

    Ok(action)
}

async fn cleanup(
    _api: &Api<AuthentikAccessPolicy>,
    policy: &AuthentikAccessPolicy,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // Deliberately unconditional: revoking a namespace's
    // AuthentikNamespacePolicy access must never block cleanup of
    // resources it already created — see .prompt/plan.md, "Securite
    // operationnelle & deploiement".
    if let Some(id) = policy
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        // Best-effort: re-resolve the owning application's instance. If
        // it's already gone (common — an application and its access
        // policies are often torn down together), fall back to the
        // single default instance rather than blocking cleanup on a
        // resolution that can no longer succeed cleanly.
        let namespace = policy.namespace().unwrap_or_default();
        let app_api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), &namespace);
        let owning_application = app_api
            .get_opt(&policy.spec.application_ref)
            .await
            .ok()
            .flatten();
        let gateway = match owning_application {
            Some(application) => {
                ctx.gateway_factory
                    .gateway_for(&application.spec.instance_ref)
                    .await
            }
            None => ctx.gateway_factory.default_gateway().await,
        }
        .map_err(|e| Error::Gateway(e.to_string()))?;

        gateway
            .delete_policy_binding(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, policy).await;
    Ok(Action::await_change())
}
