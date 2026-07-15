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
