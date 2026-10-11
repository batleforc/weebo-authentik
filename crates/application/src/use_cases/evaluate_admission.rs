//! `domain::allow_list::evaluate` applied to a `(namespace, kind)` request.
//! Shared verbatim by the webhook (`adapters-inbound::webhook`) and the
//! `AuthentikApplication`/`AuthentikAccessPolicy` reconcilers, which
//! re-check every policy through the same [`enforce`]: a policy may have
//! changed since the CR was admitted.

use api::namespace_policy::{
    Effect as ApiEffect, NamespaceRule as PolicyRule, ResourceKind as ApiResourceKind,
};
use domain::allow_list::{self, Decision, Effect, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;

use crate::ports::{ClusterError, PolicySource};
use crate::use_cases::ReconcileOutcome;

pub struct AdmissionRequest<'a> {
    pub namespace: &'a str,
    pub kind: ResourceKind,
}

pub struct AdmissionResult {
    pub allowed: bool,
    pub reason: Option<ReasonCode>,
    /// Why it was denied, naming the namespace and kind.
    pub message: Option<String>,
}

impl AdmissionResult {
    /// The outcome a reconciler publishes instead of reconciling when the
    /// CR is denied (`None`: allowed). The remote object, if any, is left
    /// alone: only the CR's deletion removes it.
    pub fn refusal(self) -> Option<ReconcileOutcome> {
        if self.allowed {
            return None;
        }
        Some(ReconcileOutcome::Errored {
            reason: self.reason.unwrap_or(allow_list::DENY_REASON),
            message: self.message.unwrap_or_else(|| "denied".to_string()),
        })
    }
}

pub fn evaluate_admission(
    request: &AdmissionRequest<'_>,
    rules: &[NamespaceRule],
    policies_exist: bool,
) -> AdmissionResult {
    match allow_list::evaluate(request.namespace, request.kind, rules, policies_exist) {
        Decision::Allowed => AdmissionResult {
            allowed: true,
            reason: None,
            message: None,
        },
        Decision::Denied(why) => AdmissionResult {
            allowed: false,
            reason: Some(allow_list::DENY_REASON),
            message: Some(why),
        },
    }
}

/// Reads every `AuthentikNamespacePolicy` through `policies` and evaluates
/// `kind` in `namespace` against them.
pub async fn enforce(
    policies: &dyn PolicySource<PolicyRule>,
    namespace: &str,
    kind: ResourceKind,
) -> Result<AdmissionResult, ClusterError> {
    let policies = policies.policies().await?;
    Ok(check(namespace, kind, &policies))
}

/// `policies`: the `spec.rules` of every `AuthentikNamespacePolicy`, one
/// list per policy — a policy with no rule still switches to default-deny.
pub fn check(namespace: &str, kind: ResourceKind, policies: &[Vec<PolicyRule>]) -> AdmissionResult {
    let rules: Vec<NamespaceRule> = policies.iter().flatten().map(rule_of).collect();
    evaluate_admission(
        &AdmissionRequest { namespace, kind },
        &rules,
        !policies.is_empty(),
    )
}

pub fn rule_of(rule: &PolicyRule) -> NamespaceRule {
    allow_list::rule(
        rule.namespaces.clone(),
        rule.allowed_kinds.iter().map(kind_of).collect(),
        match rule.effect {
            ApiEffect::Allow => Effect::Allow,
            ApiEffect::Deny => Effect::Deny,
        },
    )
}

pub fn kind_of(kind: &ApiResourceKind) -> ResourceKind {
    match kind {
        ApiResourceKind::AuthentikApplication => ResourceKind::AuthentikApplication,
        ApiResourceKind::AuthentikAccessPolicy => ResourceKind::AuthentikAccessPolicy,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use domain::allow_list::rule;

    #[test]
    fn allows_when_no_policies_exist() {
        let result = evaluate_admission(
            &AdmissionRequest {
                namespace: "team-a",
                kind: ResourceKind::AuthentikApplication,
            },
            &[],
            false,
        );
        assert!(result.allowed);
        assert_eq!(result.reason, None);
    }

    #[test]
    fn denies_by_default_once_a_policy_exists() {
        let result = evaluate_admission(
            &AdmissionRequest {
                namespace: "team-a",
                kind: ResourceKind::AuthentikApplication,
            },
            &[],
            true,
        );
        assert!(!result.allowed);
        assert_eq!(result.reason, Some(ReasonCode::NamespaceNotAllowed));
        assert!(result.message.unwrap().contains("\"team-a\""));
    }

    #[test]
    fn allows_when_explicitly_permitted() {
        let rules = [rule(
            vec!["team-a".to_string()],
            vec![ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        let result = evaluate_admission(
            &AdmissionRequest {
                namespace: "team-a",
                kind: ResourceKind::AuthentikApplication,
            },
            &rules,
            true,
        );
        assert!(result.allowed);
        assert_eq!(result.reason, None);
    }

    /// In-memory [`PolicySource`]: the policies, or the read's error.
    struct FakePolicies(Mutex<Result<Vec<Vec<PolicyRule>>, ClusterError>>);

    #[async_trait::async_trait]
    impl PolicySource<PolicyRule> for FakePolicies {
        async fn policies(&self) -> Result<Vec<Vec<PolicyRule>>, ClusterError> {
            self.0.lock().unwrap().clone()
        }
    }

    fn allow(namespace: &str, kind: ApiResourceKind) -> PolicyRule {
        PolicyRule {
            namespaces: vec![namespace.to_string()],
            allowed_kinds: vec![kind],
            effect: ApiEffect::Allow,
        }
    }

    #[tokio::test]
    async fn enforce_reads_the_policies_through_the_port() {
        let policies = FakePolicies(Mutex::new(Ok(vec![])));
        let app = ResourceKind::AuthentikApplication;
        assert!(enforce(&policies, "team-a", app).await.unwrap().allowed);

        // A policy without any rule: default-deny.
        *policies.0.lock().unwrap() = Ok(vec![vec![]]);
        let refusal = enforce(&policies, "team-a", app)
            .await
            .unwrap()
            .refusal()
            .expect("denied once a policy exists");
        assert!(matches!(
            refusal,
            ReconcileOutcome::Errored {
                reason: ReasonCode::NamespaceNotAllowed,
                ..
            }
        ));

        // Rules from several policies are merged, and stay per kind.
        *policies.0.lock().unwrap() = Ok(vec![
            vec![allow("team-b", ApiResourceKind::AuthentikApplication)],
            vec![allow("team-a", ApiResourceKind::AuthentikApplication)],
        ]);
        assert!(enforce(&policies, "team-a", app).await.unwrap().allowed);
        assert!(
            !enforce(&policies, "team-a", ResourceKind::AuthentikAccessPolicy)
                .await
                .unwrap()
                .allowed
        );

        *policies.0.lock().unwrap() = Err(ClusterError("apiserver down".into()));
        assert!(enforce(&policies, "team-a", app).await.is_err());
    }

    #[test]
    fn deny_rule_wins_across_policies() {
        let mut deny = allow("team-*", ApiResourceKind::AuthentikApplication);
        deny.effect = ApiEffect::Deny;
        let policies = vec![
            vec![allow("team-a", ApiResourceKind::AuthentikApplication)],
            vec![deny],
        ];
        let result = check("team-a", ResourceKind::AuthentikApplication, &policies);
        assert!(!result.allowed);
        assert!(result.refusal().is_some());
    }
}
