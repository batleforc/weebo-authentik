use api::AuthentikGroup;
use domain::error::ReasonCode;

use crate::ports::{AuthentikGateway, GatewayError};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

pub async fn reconcile_group(
    group: &AuthentikGroup,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway
            .update_group(id, group)
            .await
            .map(|()| id.to_string()),
        None => gateway.create_group(group).await,
    };

    match result {
        Ok(id) => ReconcileOutcome::Synced {
            authentik_id: Some(id),
        },
        // `spec.parentRef`, if set, is resolved by name inside
        // create_group/update_group — see `ports.rs`'s doc on
        // `create_group`. Same precedent as `reconcile_application`'s
        // `attach_outpost` handling: a `NotFound` from this call is
        // reported as the ref not resolving, not a generic API error.
        Err(GatewayError::NotFound(message)) => ReconcileOutcome::Errored {
            reason: ReasonCode::GroupRefNotFound,
            message,
        },
        Err(e) => errored_from_gateway_error(e),
    }
}

#[cfg(test)]
mod tests {
    use api::group::AuthentikGroupSpec;
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn group(name: &str) -> AuthentikGroup {
        AuthentikGroup {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: AuthentikGroupSpec {
                name: name.to_string(),
                is_superuser: false,
                parent_ref: None,
                attributes: Default::default(),
            },
            status: None,
        }
    }

    #[tokio::test]
    async fn first_reconcile_creates_and_syncs_the_returned_id() {
        let gateway = FakeGateway::create(Ok("11".to_string()));
        let outcome = reconcile_group(&group("weebo-user"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "11"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_and_keeps_the_existing_id() {
        let gateway = FakeGateway::update(Ok(()));
        let outcome = reconcile_group(&group("weebo-user"), Some("11"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "11"
        ));
    }

    #[tokio::test]
    async fn gateway_error_maps_to_its_reason_code() {
        let gateway = FakeGateway::create(Err(GatewayError::AlreadyExists("dup".to_string())));
        let outcome = reconcile_group(&group("weebo-user"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unresolved_parent_ref_maps_to_group_ref_not_found() {
        let gateway = FakeGateway::create(Err(GatewayError::NotFound(
            "group \"missing-parent\" not found".to_string(),
        )));
        let outcome = reconcile_group(&group("weebo-user"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::GroupRefNotFound,
                ..
            }
        ));
    }
}
