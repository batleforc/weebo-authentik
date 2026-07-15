//! Single source of truth for every reason the operator surfaces. Status
//! conditions, Kubernetes Events, and admission webhook denials all
//! reference one of these variants — never an ad-hoc string at the call
//! site. See `.prompt/plan.md`, "Convention d'erreurs : codes stables".

/// Whether a `ReasonCode` blocks `Ready` or is purely advisory (the object
/// still functions, the condition is just a stable, discoverable signal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Blocking,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonCode {
    /// Two `AuthentikBrand` CRs both request `spec.default: true`.
    DefaultBrandConflict,
    /// `AuthentikAccessPolicy.applicationRef` does not resolve to an
    /// `AuthentikApplication` in the policy's own namespace.
    ApplicationRefNotFound,
    /// Refused by `AuthentikNamespacePolicy` (webhook and/or reconciler).
    NamespaceNotAllowed,
    /// A proxy provider's `outpostRef` does not resolve to an
    /// `AuthentikOutpost`.
    OutpostRefNotFound,
    /// Create was rejected because an object with the same name/slug
    /// already exists in Authentik and isn't tracked by this CR.
    AuthentikObjectAlreadyExists,
    /// An `AuthentikApplication` is active with no `AuthentikAccessPolicy`
    /// bound to it — Authentik's native default leaves it open to any
    /// authenticated user. Advisory: `Ready` stays `True`.
    NoAccessPolicyBound,
    /// Generic success reason for `Ready: True` — every condition must
    /// carry a `ReasonCode` (see `Condition::ready`'s type-level
    /// constraint), so this is the one every reconciler uses on the happy
    /// path rather than each inventing its own "it worked" code.
    Reconciled,
    /// `AuthentikApplication.spec.provider.kind` is `saml`/`ldap` —
    /// schema-only stubs, never implemented by a reconciler. See
    /// `.prompt/plan.md`, decision 2.
    UnsupportedProviderKind,
    /// Catch-all for an `AuthentikGateway` call that failed for a reason
    /// not covered by a more specific code (network error, unexpected
    /// Authentik API response, flow/property-mapping/group name that
    /// doesn't resolve). See `application::ports::GatewayError::Api`.
    AuthentikApiError,
    /// `spec.instanceRef` does not resolve to an `AuthentikInstance` CR.
    InstanceRefNotFound,
    /// A CRD with no `instanceRef` field needed the single default
    /// `AuthentikInstance`, but the cluster has zero or more than one.
    AmbiguousDefaultInstance,
    /// Kubernetes-side failure while resolving an `AuthentikInstance` or
    /// its token `Secret` (API error, missing secret, missing/non-utf8
    /// key). See `application::ports::GatewayFactoryError::ResolutionFailed`.
    InstanceResolutionFailed,
}

impl ReasonCode {
    /// PascalCase representation, valid as both a Kubernetes `Condition`/
    /// `Event` `reason` and an admission response message prefix.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReasonCode::DefaultBrandConflict => "DefaultBrandConflict",
            ReasonCode::ApplicationRefNotFound => "ApplicationRefNotFound",
            ReasonCode::NamespaceNotAllowed => "NamespaceNotAllowed",
            ReasonCode::OutpostRefNotFound => "OutpostRefNotFound",
            ReasonCode::AuthentikObjectAlreadyExists => "AuthentikObjectAlreadyExists",
            ReasonCode::NoAccessPolicyBound => "NoAccessPolicyBound",
            ReasonCode::Reconciled => "Reconciled",
            ReasonCode::UnsupportedProviderKind => "UnsupportedProviderKind",
            ReasonCode::AuthentikApiError => "AuthentikApiError",
            ReasonCode::InstanceRefNotFound => "InstanceRefNotFound",
            ReasonCode::AmbiguousDefaultInstance => "AmbiguousDefaultInstance",
            ReasonCode::InstanceResolutionFailed => "InstanceResolutionFailed",
        }
    }

    pub const fn severity(self) -> Severity {
        match self {
            ReasonCode::NoAccessPolicyBound | ReasonCode::Reconciled => Severity::Advisory,
            _ => Severity::Blocking,
        }
    }
}

impl std::fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_is_pascal_case_no_whitespace() {
        for code in [
            ReasonCode::DefaultBrandConflict,
            ReasonCode::ApplicationRefNotFound,
            ReasonCode::NamespaceNotAllowed,
            ReasonCode::OutpostRefNotFound,
            ReasonCode::AuthentikObjectAlreadyExists,
            ReasonCode::NoAccessPolicyBound,
            ReasonCode::Reconciled,
            ReasonCode::UnsupportedProviderKind,
            ReasonCode::AuthentikApiError,
            ReasonCode::InstanceRefNotFound,
            ReasonCode::AmbiguousDefaultInstance,
            ReasonCode::InstanceResolutionFailed,
        ] {
            let s = code.as_str();
            assert!(s.chars().next().unwrap().is_ascii_uppercase());
            assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn only_no_access_policy_bound_is_advisory() {
        assert_eq!(
            ReasonCode::NoAccessPolicyBound.severity(),
            Severity::Advisory
        );
        assert_eq!(
            ReasonCode::DefaultBrandConflict.severity(),
            Severity::Blocking
        );
        assert_eq!(
            ReasonCode::NamespaceNotAllowed.severity(),
            Severity::Blocking
        );
    }
}
