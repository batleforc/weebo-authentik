use api::AuthentikFlow;

use crate::ports::AuthentikGateway;
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// A flow is a leaf resource — it references no other CRD — so this is the
/// plainest reconcile of the family: attempt-create-first on the initial
/// pass (identity keyed by the slug stored in `status.authentikId`), patch
/// in place afterwards, and map any gateway error through the shared
/// `ReasonCode` conversion. See `reconcile_group` for the same shape with
/// an added `parentRef` resolution step that flows don't need.
pub async fn reconcile_flow(
    flow: &AuthentikFlow,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway.update_flow(id, flow).await.map(|()| id.to_string()),
        None => gateway.create_flow(flow).await,
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
    use api::flow::{AuthentikFlowSpec, FlowDesignation};
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn flow(slug: &str) -> AuthentikFlow {
        AuthentikFlow {
            metadata: ObjectMeta {
                name: Some(slug.to_string()),
                ..Default::default()
            },
            spec: AuthentikFlowSpec {
                slug: slug.to_string(),
                name: slug.to_string(),
                title: "Sign in".to_string(),
                designation: FlowDesignation::Authentication,
                authentication: None,
                policy_engine_mode: None,
                compatibility_mode: None,
                layout: None,
                denied_action: None,
                background: None,
            },
            status: None,
        }
    }

    #[tokio::test]
    async fn first_reconcile_creates_and_syncs_the_returned_id() {
        let gateway = FakeGateway::create(Ok("device-code".to_string()));
        let outcome = reconcile_flow(&flow("device-code"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "device-code"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_and_keeps_the_existing_id() {
        let gateway = FakeGateway::update(Ok(()));
        let outcome = reconcile_flow(&flow("device-code"), Some("device-code"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "device-code"
        ));
    }

    #[tokio::test]
    async fn gateway_error_maps_to_its_reason_code() {
        let gateway = FakeGateway::create(Err(GatewayError::AlreadyExists("dup".to_string())));
        let outcome = reconcile_flow(&flow("device-code"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
    }
}
