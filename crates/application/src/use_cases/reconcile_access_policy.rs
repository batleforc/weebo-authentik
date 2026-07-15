use api::{AuthentikAccessPolicy, AuthentikApplication};
use domain::error::ReasonCode;

use crate::ports::AuthentikGateway;
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// `resolved_application` must have been looked up by the caller via
/// `Api::<AuthentikApplication>::namespaced(policy's own namespace)` —
/// never a cluster-wide client. `None` means the name didn't resolve in
/// that namespace, which this function turns into
/// `ReasonCode::ApplicationRefNotFound`; a same-named application in a
/// different namespace is invisible by construction. See
/// `.prompt/plan.md`, "Confinement au namespace, deux couches".
///
/// `gateway` is `Option` because the caller only knows which
/// `AuthentikInstance` to resolve a gateway for once it has the
/// application in hand (`AuthentikAccessPolicy` itself carries no
/// `instanceRef`) — when `resolved_application` is `None` there is
/// nothing to resolve a gateway against either, so the caller passes
/// `None` rather than an unrelated fallback instance.
pub async fn reconcile_access_policy(
    policy: &AuthentikAccessPolicy,
    resolved_application: Option<&AuthentikApplication>,
    authentik_id: Option<&str>,
    gateway: Option<&dyn AuthentikGateway>,
) -> ReconcileOutcome {
    let Some(application) = resolved_application else {
        return ReconcileOutcome::Errored {
            reason: ReasonCode::ApplicationRefNotFound,
            message: format!(
                "applicationRef {:?} not found in namespace {:?}",
                policy.spec.application_ref,
                policy.metadata.namespace.as_deref().unwrap_or(""),
            ),
        };
    };
    let gateway = gateway.expect(
        "gateway is Some whenever resolved_application is Some, by the caller's construction",
    );

    // The application must itself already be reconciled (own an
    // `authentik_id`, i.e. a slug) before a binding can target it.
    let Some(application_authentik_id) = application
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    else {
        return ReconcileOutcome::Errored {
            reason: ReasonCode::ApplicationRefNotFound,
            message: format!(
                "applicationRef {:?} exists but has not synced to Authentik yet",
                policy.spec.application_ref,
            ),
        };
    };

    let result = match authentik_id {
        Some(id) => gateway
            .upsert_policy_binding(
                Some(id),
                application_authentik_id,
                &policy.spec.group_ref,
                policy.spec.order,
                policy.spec.negate,
            )
            .await
            .map(|_| id.to_string()),
        None => {
            gateway
                .upsert_policy_binding(
                    None,
                    application_authentik_id,
                    &policy.spec.group_ref,
                    policy.spec.order,
                    policy.spec.negate,
                )
                .await
        }
    };

    match result {
        Ok(id) => ReconcileOutcome::Synced {
            authentik_id: Some(id),
        },
        Err(e) => errored_from_gateway_error(e),
    }
}
