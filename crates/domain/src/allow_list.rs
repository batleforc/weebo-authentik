//! Pure evaluation of `AuthentikNamespacePolicy` rules. Shared verbatim by
//! the admission webhook and any reconciler that needs to re-check policy.
//! See `.prompt/plan.md`, the `AuthentikNamespacePolicy` CRD section.

use crate::error::ReasonCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    AuthentikApplication,
    AuthentikAccessPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct NamespaceRule {
    pub namespaces: Vec<String>,
    pub allowed_kinds: Vec<ResourceKind>,
    pub effect: Effect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Denied,
}

/// The reason attached whenever `evaluate` returns `Denied`.
pub const DENY_REASON: ReasonCode = ReasonCode::NamespaceNotAllowed;

/// Default-deny: `(namespace, kind)` is allowed only if at least one rule
/// explicitly allows it. Any matching `Deny` rule wins over any matching
/// `Allow` rule regardless of list order — a narrower deny always overrides
/// a broader allow, never the other way around.
pub fn evaluate(namespace: &str, kind: ResourceKind, rules: &[NamespaceRule]) -> Decision {
    let mut decision = Decision::Denied;
    for rule in rules {
        let matches =
            rule.namespaces.iter().any(|n| n == namespace) && rule.allowed_kinds.contains(&kind);
        if !matches {
            continue;
        }
        match rule.effect {
            Effect::Deny => return Decision::Denied,
            Effect::Allow => decision = Decision::Allowed,
        }
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(ns: &[&str], kinds: &[ResourceKind], effect: Effect) -> NamespaceRule {
        NamespaceRule {
            namespaces: ns.iter().map(|s| s.to_string()).collect(),
            allowed_kinds: kinds.to_vec(),
            effect,
        }
    }

    #[test]
    fn default_deny_with_no_rules() {
        let decision = evaluate("team-a", ResourceKind::AuthentikApplication, &[]);
        assert_eq!(decision, Decision::Denied);
    }

    #[test]
    fn explicit_allow_matches() {
        let rules = [rule(
            &["team-a"],
            &[ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        assert_eq!(
            evaluate("team-a", ResourceKind::AuthentikApplication, &rules),
            Decision::Allowed
        );
        assert_eq!(
            evaluate("team-b", ResourceKind::AuthentikApplication, &rules),
            Decision::Denied
        );
    }

    #[test]
    fn allow_does_not_leak_across_kinds() {
        let rules = [rule(
            &["team-a"],
            &[ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        assert_eq!(
            evaluate("team-a", ResourceKind::AuthentikAccessPolicy, &rules),
            Decision::Denied
        );
    }

    #[test]
    fn deny_wins_over_allow_regardless_of_order() {
        let kinds = [ResourceKind::AuthentikApplication];
        let allow_then_deny = [
            rule(&["team-a"], &kinds, Effect::Allow),
            rule(&["team-a"], &kinds, Effect::Deny),
        ];
        assert_eq!(
            evaluate(
                "team-a",
                ResourceKind::AuthentikApplication,
                &allow_then_deny
            ),
            Decision::Denied
        );

        let deny_then_allow = [
            rule(&["team-a"], &kinds, Effect::Deny),
            rule(&["team-a"], &kinds, Effect::Allow),
        ];
        assert_eq!(
            evaluate(
                "team-a",
                ResourceKind::AuthentikApplication,
                &deny_then_allow
            ),
            Decision::Denied
        );
    }
}
