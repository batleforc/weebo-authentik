use api::AuthentikUser;
use domain::error::ReasonCode;

use crate::ports::{AuthentikGateway, GatewayError};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// Note the CR carries no credential field by design (see `api::user`) so
/// this use-case never touches a password.
pub async fn reconcile_user(
    user: &AuthentikUser,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway.update_user(id, user).await.map(|()| id.to_string()),
        None => gateway.create_user(user).await,
    };

    match result {
        Ok(id) => ReconcileOutcome::Synced {
            authentik_id: Some(id),
        },
        // `spec.groupRefs` entries are resolved by name inside
        // create_user/update_user — see `ports.rs`'s doc on `create_user`.
        // Same precedent as `reconcile_application`'s `attach_outpost`
        // handling: a `NotFound` from this call is reported as a group ref
        // not resolving, not a generic API error.
        Err(GatewayError::NotFound(message)) => ReconcileOutcome::Errored {
            reason: ReasonCode::GroupRefNotFound,
            message,
        },
        Err(e) => errored_from_gateway_error(e),
    }
}

#[cfg(test)]
mod tests {
    use api::user::AuthentikUserSpec;
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn user(name: &str) -> AuthentikUser {
        AuthentikUser {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: AuthentikUserSpec {
                username: name.to_string(),
                name: name.to_string(),
                email: format!("{name}@example.com"),
                is_active: true,
                group_refs: vec![],
            },
            status: None,
        }
    }

    #[tokio::test]
    async fn first_reconcile_creates_and_syncs_the_returned_id() {
        let gateway = FakeGateway::create(Ok("3".to_string()));
        let outcome = reconcile_user(&user("batleforc"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "3"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_and_keeps_the_existing_id() {
        let gateway = FakeGateway::update(Ok(()));
        let outcome = reconcile_user(&user("batleforc"), Some("3"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "3"
        ));
    }

    #[tokio::test]
    async fn unresolved_group_ref_maps_to_group_ref_not_found() {
        let gateway = FakeGateway::create(Err(GatewayError::NotFound("group missing".to_string())));
        let outcome = reconcile_user(&user("batleforc"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::GroupRefNotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn other_gateway_errors_map_to_the_generic_reason_code() {
        let gateway = FakeGateway::create(Err(GatewayError::AlreadyExists("dup".to_string())));
        let outcome = reconcile_user(&user("batleforc"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
    }
}
