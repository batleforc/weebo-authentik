use std::sync::Arc;

use api::{AuthentikAccessPolicy, AuthentikApplication};
use application::ports::BoundInstance;
use application::use_cases::reconcile_access_policy::{
    application_ref_not_found, reconcile_access_policy,
};
use application::use_cases::{errored_from_factory_error, errored_from_instance_error};
use domain::Failure;
use domain::allow_list::ResourceKind;
use domain::error::ReasonCode;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikAccessPolicy>(client.clone());
    let health = ctx.health.clone();
    let results = super::on_policy_change(super::controller(api, &ctx.scope), client, &ctx.scope)
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
    let current = super::current(api, policy).await?;
    let policy = &current;
    let authentik_id = policy.status.as_ref().and_then(|s| s.authentik_id.clone());

    // Same-namespace lookup, never a cluster-wide client — this is the
    // enforcement mechanism from `.prompt/plan.md`, "Confinement au
    // namespace, deux couches": a same-named AuthentikApplication in a
    // different namespace is invisible here by construction.
    let app_api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), namespace);
    let application = app_api.get_opt(&policy.spec.application_ref).await?;
    // The policy lives on its application's instance: the one the
    // application's status records, or its `instanceRef` resolved.
    let bound = match &application {
        Some(application) => Some(
            match application.status.as_ref().and_then(|s| s.instance.clone()) {
                Some(bound) => Ok(bound),
                None => super::application_instance(ctx, application).await,
            },
        ),
        None => None,
    };
    let instance = bound.as_ref().and_then(|b| b.as_ref().ok()).cloned();
    let refusal = super::policy_refusal(
        ctx,
        namespace,
        ResourceKind::AuthentikAccessPolicy,
        instance.as_ref(),
    )
    .await?;
    let outcome = match (refusal, application, bound) {
        // Re-checked here too: a policy may have changed since admission.
        (Some(refused), _, _) => refused,
        (None, Some(application), Some(Ok(bound))) => {
            match ctx.gateway_factory.gateway_for(&bound).await {
                Ok(gateway) => {
                    reconcile_access_policy(
                        policy,
                        &application,
                        authentik_id.as_deref(),
                        gateway.as_ref(),
                    )
                    .await
                }
                Err(e) => errored_from_factory_error(e),
            }
        }
        (None, Some(_), Some(Err(e))) => errored_from_instance_error(e),
        (None, _, _) => application_ref_not_found(policy),
    };
    super::record_reconcile("AuthentikAccessPolicy", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome_on(
        api,
        &ctx.recorder,
        policy,
        outcome,
        "access policy synced",
        instance.as_ref(),
    )
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
        let gateway = match policy_instance(policy, ctx).await? {
            Some(bound) => ctx.gateway_factory.gateway_for(&bound).await,
            None => ctx.gateway_factory.default_gateway().await,
        }
        .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;

        gateway
            .delete_policy_binding(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, policy).await;
    Ok(Action::await_change())
}

/// The instance a policy's binding lives on: the one recorded in its
/// status (every synced policy records it); else — a policy synced before
/// the field existed — its application's. `None` (the single default
/// instance) only when neither is known any more *and* the operator has
/// cluster-scoped instances: an application whose instance can't be
/// resolved, or an operator limited to local instances, has no default to
/// guess, and deleting on the wrong instance would orphan the binding
/// silently (a 404 there counts as already deleted).
async fn policy_instance(
    policy: &AuthentikAccessPolicy,
    ctx: &Ctx,
) -> Result<Option<BoundInstance>, Error> {
    if let Some(recorded) = policy.status.as_ref().and_then(|s| s.instance.clone()) {
        return Ok(Some(recorded));
    }
    let namespace = policy.namespace().unwrap_or_default();
    let app_api: Api<AuthentikApplication> = Api::namespaced(ctx.client.clone(), &namespace);
    if let Some(application) = app_api.get_opt(&policy.spec.application_ref).await? {
        return match application.status.as_ref().and_then(|s| s.instance.clone()) {
            Some(bound) => Ok(Some(bound)),
            None => super::application_instance(ctx, &application)
                .await
                .map(Some)
                .map_err(|e| Error::cleanup(errored_from_instance_error(e))),
        };
    }
    if ctx.scope.cluster_kinds() {
        return Ok(None);
    }
    Err(Error::Cleanup(Failure::new(
        ReasonCode::InstanceRefNotFound,
        format!(
            "neither the status nor application {:?} says which instance holds binding {}",
            policy.spec.application_ref,
            policy
                .status
                .as_ref()
                .and_then(|s| s.authentik_id.as_deref())
                .unwrap_or_default()
        ),
    )))
}
