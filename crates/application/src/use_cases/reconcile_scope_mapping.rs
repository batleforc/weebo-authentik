use api::AuthentikScopeMapping;

use crate::ports::AuthentikGateway;
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// Create-then-update, keyed on `status.authentikId`, same shape as
/// `reconcile_flow`. A scope mapping references no other CRD — no group,
/// flow or certificate name to resolve — so there is no
/// `GatewayError::NotFound`-as-unresolved-ref arm here: a `NotFound` from
/// the gateway can only be the mapping's own pk having disappeared
/// upstream, which is a plain API error.
pub async fn reconcile_scope_mapping(
    mapping: &AuthentikScopeMapping,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway
            .update_scope_mapping(id, mapping)
            .await
            .map(|()| id.to_string()),
        None => gateway.create_scope_mapping(mapping).await,
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
    use api::scope_mapping::AuthentikScopeMappingSpec;
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn mapping(name: &str) -> AuthentikScopeMapping {
        AuthentikScopeMapping {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: AuthentikScopeMappingSpec {
                name: name.to_string(),
                scope_name: "groups".to_string(),
                expression: "return {\"groups\": []}".to_string(),
                description: None,
            },
            status: None,
        }
    }

    #[tokio::test]
    async fn first_reconcile_creates_and_syncs_the_returned_id() {
        let gateway = FakeGateway::create(Ok("f0d4d2b0-0000-4000-8000-000000000001".to_string()));
        let outcome = reconcile_scope_mapping(&mapping("rustfs-policies"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) }
                if id == "f0d4d2b0-0000-4000-8000-000000000001"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_and_keeps_the_existing_id() {
        let gateway = FakeGateway::update(Ok(()));
        let outcome =
            reconcile_scope_mapping(&mapping("rustfs-policies"), Some("11"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "11"
        ));
    }

    #[tokio::test]
    async fn name_collision_maps_to_already_exists() {
        let gateway = FakeGateway::create(Err(GatewayError::AlreadyExists("dup".to_string())));
        let outcome = reconcile_scope_mapping(&mapping("rustfs-policies"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
    }

    /// Unlike `reconcile_group`, a `NotFound` here is not a dangling ref —
    /// there is nothing to resolve — so it stays a generic API error.
    #[tokio::test]
    async fn not_found_stays_an_api_error() {
        let gateway = FakeGateway::update(Err(GatewayError::NotFound("gone".to_string())));
        let outcome =
            reconcile_scope_mapping(&mapping("rustfs-policies"), Some("11"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }
}
