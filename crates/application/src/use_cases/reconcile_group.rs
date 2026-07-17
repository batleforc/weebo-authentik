use api::AuthentikGroup;

use crate::ports::AuthentikGateway;
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
}
