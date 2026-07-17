use api::AuthentikOutpost;

use crate::ports::AuthentikGateway;
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// Note this CRD is deliberately minimal (see `api::outpost`) since
/// nothing in the Terraform module being replaced creates a custom
/// outpost — most proxy providers attach to the embedded outpost instead
/// (see `reconcile_application`, `AuthentikGateway::attach_outpost`).
pub async fn reconcile_outpost(
    outpost: &AuthentikOutpost,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway
            .update_outpost(id, outpost)
            .await
            .map(|()| id.to_string()),
        None => gateway.create_outpost(outpost).await,
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
    use api::outpost::{AuthentikOutpostSpec, OutpostType};
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    fn outpost(name: &str) -> AuthentikOutpost {
        AuthentikOutpost {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: AuthentikOutpostSpec {
                name: name.to_string(),
                r#type: OutpostType::Proxy,
                config: serde_json::json!({}),
            },
            status: None,
        }
    }

    #[tokio::test]
    async fn first_reconcile_creates_and_syncs_the_returned_id() {
        let gateway = FakeGateway::create(Ok("7".to_string()));
        let outcome = reconcile_outpost(&outpost("edge"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "7"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_and_keeps_the_existing_id() {
        let gateway = FakeGateway::update(Ok(()));
        let outcome = reconcile_outpost(&outpost("edge"), Some("7"), &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "7"
        ));
    }

    #[tokio::test]
    async fn gateway_error_maps_to_its_reason_code() {
        let gateway = FakeGateway::create(Err(GatewayError::Api("boom".to_string())));
        let outcome = reconcile_outpost(&outpost("edge"), None, &gateway).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }
}
