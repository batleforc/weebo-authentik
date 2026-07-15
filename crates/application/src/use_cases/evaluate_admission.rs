//! Fully implemented, not a stub — this is `domain::allow_list::evaluate`
//! applied to an incoming admission request. Shared verbatim by the
//! webhook (`adapters-inbound::webhook`) and any reconciler that needs to
//! re-check policy against the same rules.

use domain::allow_list::{self, Decision, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;

pub struct AdmissionRequest<'a> {
    pub namespace: &'a str,
    pub kind: ResourceKind,
}

pub struct AdmissionResult {
    pub allowed: bool,
    pub reason: Option<ReasonCode>,
}

pub fn evaluate_admission(
    request: &AdmissionRequest<'_>,
    rules: &[NamespaceRule],
) -> AdmissionResult {
    match allow_list::evaluate(request.namespace, request.kind, rules) {
        Decision::Allowed => AdmissionResult {
            allowed: true,
            reason: None,
        },
        Decision::Denied => AdmissionResult {
            allowed: false,
            reason: Some(allow_list::DENY_REASON),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::allow_list::Effect;

    #[test]
    fn denies_by_default() {
        let result = evaluate_admission(
            &AdmissionRequest {
                namespace: "team-a",
                kind: ResourceKind::AuthentikApplication,
            },
            &[],
        );
        assert!(!result.allowed);
        assert_eq!(result.reason, Some(ReasonCode::NamespaceNotAllowed));
    }

    #[test]
    fn allows_when_explicitly_permitted() {
        let rules = [NamespaceRule {
            namespaces: vec!["team-a".to_string()],
            allowed_kinds: vec![ResourceKind::AuthentikApplication],
            effect: Effect::Allow,
        }];
        let result = evaluate_admission(
            &AdmissionRequest {
                namespace: "team-a",
                kind: ResourceKind::AuthentikApplication,
            },
            &rules,
        );
        assert!(result.allowed);
        assert_eq!(result.reason, None);
    }
}
