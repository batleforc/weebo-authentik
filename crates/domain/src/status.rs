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
