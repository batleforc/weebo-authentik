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

#[cfg(test)]
mod tests {
    use api::access_policy::AuthentikAccessPolicySpec;
    use api::application::{
        AuthentikApplicationSpec, Oauth2ProviderSpec, ProviderKind, ProviderSpec,
    };
    use api::status::AuthentikStatus;
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn policy() -> AuthentikAccessPolicy {
        AuthentikAccessPolicy {
            metadata: ObjectMeta {
                name: Some("harbor-access".to_string()),
                namespace: Some("team-a".to_string()),
                ..Default::default()
            },
            spec: AuthentikAccessPolicySpec {
                application_ref: "harbor".to_string(),
                group_ref: "weebo-user".to_string(),
                order: 0,
                negate: false,
            },
            status: None,
        }
    }

    fn application(authentik_id: Option<&str>) -> AuthentikApplication {
        AuthentikApplication {
            metadata: ObjectMeta {
                name: Some("harbor".to_string()),
                namespace: Some("team-a".to_string()),
                ..Default::default()
            },
            spec: AuthentikApplicationSpec {
                instance_ref: "prod".to_string(),
                name: "harbor".to_string(),
                slug: "harbor".to_string(),
                meta_icon: None,
                provider: ProviderSpec {
                    kind: ProviderKind::Oauth2,
                    oauth2: Some(Oauth2ProviderSpec {
                        client_id: None,
                        authorization_flow: "default-authorization-flow".to_string(),
                        invalidation_flow: "default-invalidation-flow".to_string(),
                        signing_key: None,
                        allowed_redirect_uris: vec![],
                        property_mappings: vec![],
                        grant_types: vec![],
                    }),
                    proxy: None,
                },
            },
            status: authentik_id.map(|id| AuthentikStatus {
                authentik_id: Some(id.to_string()),
                ..Default::default()
            }),
        }
    }

    #[tokio::test]
    async fn unresolved_application_ref_errors_without_touching_the_gateway() {
        let outcome = reconcile_access_policy(&policy(), None, None, None).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::ApplicationRefNotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn application_not_yet_synced_to_authentik_errors_without_touching_the_gateway() {
        let app = application(None);
        let gateway = FakeGateway::default();
        let outcome = reconcile_access_policy(&policy(), Some(&app), None, Some(&gateway)).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::ApplicationRefNotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn first_reconcile_creates_the_binding_and_syncs_its_id() {
        let app = application(Some("harbor"));
        let gateway = FakeGateway::upsert_policy_binding(Ok("99".to_string()));
        let outcome = reconcile_access_policy(&policy(), Some(&app), None, Some(&gateway)).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "99"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_the_existing_binding() {
        let app = application(Some("harbor"));
        let gateway = FakeGateway::upsert_policy_binding(Ok("99".to_string()));
        let outcome =
            reconcile_access_policy(&policy(), Some(&app), Some("99"), Some(&gateway)).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "99"
        ));
    }

    #[tokio::test]
    async fn gateway_error_maps_to_its_reason_code() {
        let app = application(Some("harbor"));
        let gateway =
            FakeGateway::upsert_policy_binding(Err(GatewayError::NotFound("group".to_string())));
        let outcome = reconcile_access_policy(&policy(), Some(&app), None, Some(&gateway)).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }
}
