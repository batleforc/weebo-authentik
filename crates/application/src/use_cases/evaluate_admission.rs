//! `domain::allow_list::evaluate` applied to an incoming admission request.
//! Shared verbatim by the webhook (`adapters-inbound::webhook`) and any
//! reconciler that needs to re-check policy against the same rules.

use domain::allow_list::{self, Decision, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;

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

#[cfg(test)]
mod tests {
    use super::*;
    use domain::allow_list::{Effect, rule};

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
}
