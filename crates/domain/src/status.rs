//! Pure status/condition value types. Mirrored onto the Kubernetes status
//! subresource by `api`'s `AuthentikStatus`. See `.prompt/plan.md`,
//! "Modele de status commun".

use crate::error::{ReasonCode, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub type_: String,
    pub status: ConditionStatus,
    pub reason: ReasonCode,
    pub message: String,
}

impl Condition {
    /// The main `Ready` condition. Blocking reasons should always pair with
    /// `ConditionStatus::False` here.
    pub fn ready(status: ConditionStatus, reason: ReasonCode, message: impl Into<String>) -> Self {
        Self {
            type_: "Ready".to_string(),
            status,
            reason,
            message: message.into(),
        }
    }

    /// A non-blocking advisory condition alongside `Ready: True`. Panics in
    /// debug builds if given a `Blocking` reason — advisories and blocking
    /// failures are not interchangeable.
    pub fn advisory(
        type_: impl Into<String>,
        reason: ReasonCode,
        message: impl Into<String>,
    ) -> Self {
        debug_assert_eq!(
            reason.severity(),
            Severity::Advisory,
            "Condition::advisory called with a Blocking ReasonCode"
        );
        Self {
            type_: type_.into(),
            status: ConditionStatus::True,
            reason,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_builds_the_ready_condition() {
        let c = Condition::ready(ConditionStatus::True, ReasonCode::Reconciled, "synced");
        assert_eq!(c.type_, "Ready");
        assert_eq!(c.status, ConditionStatus::True);
        assert_eq!(c.reason, ReasonCode::Reconciled);
        assert_eq!(c.message, "synced");
    }

    #[test]
    fn advisory_builds_a_true_status_condition_with_the_given_type() {
        let c = Condition::advisory(
            "NoAccessPolicyBound",
            ReasonCode::NoAccessPolicyBound,
            "no policy bound yet",
        );
        assert_eq!(c.type_, "NoAccessPolicyBound");
        assert_eq!(c.status, ConditionStatus::True);
        assert_eq!(c.reason, ReasonCode::NoAccessPolicyBound);
    }

    #[test]
    #[should_panic(expected = "Blocking ReasonCode")]
    fn advisory_panics_on_a_blocking_reason_code() {
        Condition::advisory(
            "Bogus",
            ReasonCode::NamespaceNotAllowed,
            "blocking reasons must never be advisory",
        );
    }
}
