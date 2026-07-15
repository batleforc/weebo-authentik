use std::sync::Arc;

use api::{AuthentikAccessPolicy, AuthentikApplication};
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_access_policy::reconcile_access_policy;
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
    let api: Api<AuthentikAccessPolicy> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikAccessPolicy reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikAccessPolicy>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let ns = obj.namespace().unwrap_or_default();
    let api: Api<AuthentikAccessPolicy> = Api::namespaced(ctx.client.clone(), &ns);

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(policy) => apply(&api, &policy, &ctx, &ns).await,
            FinalizerEvent::Cleanup(policy) => cleanup(&api, &policy, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikAccessPolicy>,
    policy: &AuthentikAccessPolicy,
    ctx: &Ctx,
    namespace: &str,
) -> Result<Action, Error> {
    let name = policy.name_any();
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
                        resolved.as_ref(),
                        authentik_id.as_deref(),
                        Some(gateway.as_ref()),
                    )
                    .await
                }
                Err(e) => errored_from_factory_error(e),
            }
        }
        None => reconcile_access_policy(policy, None, authentik_id.as_deref(), None).await,
    };

    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => {
            patch_synced_status(api, &name, &id, "access policy synced").await?;
        }
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "access policy synced",
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
    Ok(Action::await_change())
}

fn error_policy(_obj: Arc<AuthentikAccessPolicy>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
