use api::AuthentikNamespacePolicy;

use crate::use_cases::ReconcileOutcome;

/// `AuthentikNamespacePolicy` has no Authentik-side object either — it's
/// pure Kubernetes-side config consumed by the admission webhook
/// (`evaluate_admission`) and any reconciler that re-checks policy.
/// Real, not a stub: always `Synced` once the API server has accepted the
/// CR, there's nothing further to reconcile remotely.
pub fn reconcile_namespace_policy(_policy: &AuthentikNamespacePolicy) -> ReconcileOutcome {
    ReconcileOutcome::Synced { authentik_id: None }
}

#[cfg(test)]
mod tests {
    use api::namespace_policy::AuthentikNamespacePolicySpec;
    use kube::api::ObjectMeta;

    use super::*;

    #[test]
    fn accepted_cr_is_always_synced_with_no_remote_id() {
        let policy = AuthentikNamespacePolicy {
            metadata: ObjectMeta {
                name: Some("default-deny".to_string()),
                ..Default::default()
            },
            spec: AuthentikNamespacePolicySpec { rules: vec![] },
            status: None,
        };

        assert!(matches!(
            reconcile_namespace_policy(&policy),
            ReconcileOutcome::Synced { authentik_id: None }
        ));
    }
}
