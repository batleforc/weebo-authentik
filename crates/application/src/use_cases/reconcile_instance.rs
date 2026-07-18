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

#[cfg(test)]
mod tests {
    use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
    use kube::api::ObjectMeta;

    use super::*;

    #[test]
    fn accepted_cr_is_always_synced_with_no_remote_id() {
        let instance = AuthentikInstance {
            metadata: ObjectMeta {
                name: Some("prod".to_string()),
                ..Default::default()
            },
            spec: AuthentikInstanceSpec {
                url: "https://login.example.com".to_string(),
                token_secret_ref: SecretKeyRef {
                    name: "authentik-token".to_string(),
                    namespace: "authentik".to_string(),
                    key: "token".to_string(),
                },
                tls: Default::default(),
                secret_store: Default::default(),
            },
            status: None,
        };

        assert!(matches!(
            reconcile_instance(&instance),
            ReconcileOutcome::Synced { authentik_id: None }
        ));
    }
}
