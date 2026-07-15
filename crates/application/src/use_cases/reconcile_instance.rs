use api::AuthentikInstance;

use crate::use_cases::ReconcileOutcome;

/// `AuthentikInstance` has no corresponding Authentik-side object to
/// create — it just describes how to reach one. Real, not a stub: there's
/// nothing further to reconcile at this layer once the API server has
/// accepted the CR (required fields present). Actual connectivity/token
/// validation happens lazily, per call, in `GatewayFactory`
/// (`adapters_outbound::AuthentikGatewayFactory`).
pub fn reconcile_instance(_instance: &AuthentikInstance) -> ReconcileOutcome {
    ReconcileOutcome::Synced { authentik_id: None }
}
