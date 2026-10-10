//! `AuthentikNamespacePolicy` rules, evaluated by the kit's allow-list
//! engine — shared verbatim by the admission webhook and any reconciler
//! that needs to re-check policy. See `.prompt/plan.md`, the
//! `AuthentikNamespacePolicy` CRD section.
//!
//! Authentik scopes rules by namespace and kind only: no reference
//! dimension, so a rule's `scopes` and a request's `references` stay empty.

use crate::error::ReasonCode;

pub use weebo_kit_domain::allow_list::{Decision, Effect};

/// The kinds an `AuthentikNamespacePolicy` governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    AuthentikApplication,
    AuthentikAccessPolicy,
}

pub type NamespaceRule = weebo_kit_domain::allow_list::Rule<ResourceKind>;

/// Named in denial messages.
pub const POLICY_KIND: &str = "AuthentikNamespacePolicy";

/// The reason attached whenever `evaluate` returns `Denied`.
pub const DENY_REASON: ReasonCode = ReasonCode::NamespaceNotAllowed;

/// A rule over `namespaces` (exact names, `*`, or a prefix glob such as
/// `team-*`) and `kinds`.
pub fn rule(namespaces: Vec<String>, kinds: Vec<ResourceKind>, effect: Effect) -> NamespaceRule {
    NamespaceRule {
        namespaces,
        kinds,
        scopes: Default::default(),
        effect,
    }
}

/// Default-deny, *unless no `AuthentikNamespacePolicy` exists in the
/// cluster at all* — in that case every namespace is allowed, since there
/// is no policy to enforce yet. `policies_exist` must reflect whether any
/// `AuthentikNamespacePolicy` object was found, not merely whether `rules`
/// is non-empty (a policy with zero rules still counts as "exists" and
/// keeps default-deny in force).
///
/// Once at least one policy exists, `(namespace, kind)` is allowed only if
/// at least one rule explicitly allows it. Any matching `Deny` rule wins
/// over any matching `Allow` rule regardless of list order.
pub fn evaluate(
    namespace: &str,
    kind: ResourceKind,
    rules: &[NamespaceRule],
    policies_exist: bool,
) -> Decision {
    let request = weebo_kit_domain::allow_list::Request {
        namespace,
        kind,
        references: Default::default(),
    };
    weebo_kit_domain::allow_list::evaluate(POLICY_KIND, &request, rules, policies_exist)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(ns: &[&str], kinds: &[ResourceKind], effect: Effect) -> NamespaceRule {
        rule(
            ns.iter().map(|s| s.to_string()).collect(),
            kinds.to_vec(),
            effect,
        )
    }

    fn denied(d: &Decision) -> bool {
        matches!(d, Decision::Denied(_))
    }

    #[test]
    fn allowed_when_no_policies_exist_at_all() {
        let decision = evaluate("team-a", ResourceKind::AuthentikApplication, &[], false);
        assert_eq!(decision, Decision::Allowed);
    }

    #[test]
    fn default_deny_once_a_policy_exists_even_with_no_matching_rules() {
        // A policy exists but has no rules covering this namespace/kind —
        // still default-deny, distinct from the "no policy at all" case.
        let decision = evaluate("team-a", ResourceKind::AuthentikApplication, &[], true);
        assert_eq!(
            decision,
            Decision::Denied(
                "no AuthentikNamespacePolicy rule allows AuthentikApplication in namespace \"team-a\""
                    .to_string()
            )
        );
    }

    #[test]
    fn explicit_allow_matches() {
        let rules = [r(
            &["team-a"],
            &[ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        assert_eq!(
            evaluate("team-a", ResourceKind::AuthentikApplication, &rules, true),
            Decision::Allowed
        );
        assert!(denied(&evaluate(
            "team-b",
            ResourceKind::AuthentikApplication,
            &rules,
            true
        )));
    }

    #[test]
    fn namespace_patterns_are_prefix_globs() {
        let rules = [r(
            &["team-*"],
            &[ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        assert_eq!(
            evaluate("team-a", ResourceKind::AuthentikApplication, &rules, true),
            Decision::Allowed
        );
        assert!(denied(&evaluate(
            "ops",
            ResourceKind::AuthentikApplication,
            &rules,
            true
        )));
    }

    #[test]
    fn allow_does_not_leak_across_kinds() {
        let rules = [r(
            &["team-a"],
            &[ResourceKind::AuthentikApplication],
            Effect::Allow,
        )];
        assert!(denied(&evaluate(
            "team-a",
            ResourceKind::AuthentikAccessPolicy,
            &rules,
            true
        )));
    }

    #[test]
    fn deny_wins_over_allow_regardless_of_order() {
        let kinds = [ResourceKind::AuthentikApplication];
        let allow_then_deny = [
            r(&["team-a"], &kinds, Effect::Allow),
            r(&["team-a"], &kinds, Effect::Deny),
        ];
        assert!(denied(&evaluate(
            "team-a",
            ResourceKind::AuthentikApplication,
            &allow_then_deny,
            true
        )));

        let deny_then_allow = [
            r(&["team-a"], &kinds, Effect::Deny),
            r(&["team-a"], &kinds, Effect::Allow),
        ];
        assert!(denied(&evaluate(
            "team-a",
            ResourceKind::AuthentikApplication,
            &deny_then_allow,
            true
        )));
    }
}
