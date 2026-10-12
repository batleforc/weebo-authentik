//! Single source of truth for every reason the operator surfaces. Status
//! conditions, Kubernetes Events, and admission webhook denials all
//! reference one of these variants — never an ad-hoc string at the call
//! site. See `.prompt/plan.md`, "Convention d'erreurs : codes stables".

pub use weebo_kit_domain::{Reason, Severity};

weebo_kit_domain::reason_codes! {
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
        /// An `AuthentikGroup.spec.parentRef`, `AuthentikUser.spec.groupRefs`
        /// entry, or `AuthentikAccessPolicy.spec.groupRef` does not resolve to
        /// an `AuthentikGroup`'s Authentik-side name.
        GroupRefNotFound,
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
        /// `AuthentikApplication.spec.provider.kind` is `oauth2`/`proxy` but
        /// the matching `spec.provider.oauth2`/`spec.provider.proxy` field
        /// isn't set. Schema-valid (kube-core can't express "exactly one of"
        /// as a hard constraint — see `api::application::ProviderSpec`'s doc),
        /// but not a shape any reconciler can act on.
        InvalidProviderSpec,
        /// Catch-all for an `AuthentikGateway` call that failed for a reason
        /// not covered by a more specific code (network error, unexpected
        /// Authentik API response, flow/property-mapping/group name that
        /// doesn't resolve). See `application::ports::GatewayError::Api`.
        AuthentikApiError,
        /// `spec.instanceRef` does not resolve to an `AuthentikInstance` nor
        /// an `AuthentikLocalInstance` the operator sees (or can't be
        /// satisfied as written).
        InstanceRefNotFound,
        /// The instance named is out of reach: an `AuthentikLocalInstance` of
        /// another namespace that doesn't share itself with this one, or
        /// outside the namespace the operator is limited to.
        InstanceNotAllowed,
        /// `spec.instanceRef` by name now designates another instance than
        /// the one recorded in `status.instance`: not followed until
        /// `spec.instanceRef.kind` says which.
        InstanceBindingChanged,
        /// An update would take a CR still carrying the finalizer out of the
        /// operator's label selector (`watch.labelSelector`): refused at
        /// admission.
        LeavesWatchScope,
        /// A CRD with no `instanceRef` field needed the single default
        /// `AuthentikInstance`, but the cluster has zero or more than one.
        AmbiguousDefaultInstance,
        /// Kubernetes-side failure while resolving an `AuthentikInstance` or
        /// its token `Secret` (API error, missing secret, missing/non-utf8
        /// key). See `application::ports::GatewayFactoryError::ResolutionFailed`.
        InstanceResolutionFailed,
        /// A `SecretStore` (Kubernetes or Vault backend) write/delete/auth
        /// call failed — distinct from `AuthentikApiError` since it's a
        /// separate failure surface (a Vault auth/KV error is not an
        /// Authentik API error). See `application::ports::SecretStoreError`
        /// and `SecretStoreFactoryError`.
        SecretStoreError,
        /// The remote Authentik object was created for this CR (Event
        /// only: `status.authentikId` was just recorded).
        Created,
        /// The finalizer deleted the remote Authentik object of this CR
        /// (Event only).
        Deleted,
        /// The finalizer finished — cleanup done, or the CR force-orphaned
        /// (Event only).
        Released,
        /// A Kubernetes API call of the operator itself failed (reading a CR,
        /// patching its status, adding or removing its finalizer) — the
        /// reconcile stopped before reaching a status, and is retried.
        KubernetesApiError,
        /// `AuthentikApplication.spec.provider.kind` changed from what's
        /// currently live in Authentik, but the CR lacks the
        /// `authentik.weebo.io/allow-disruptive-update` annotation authorizing
        /// the delete-and-recreate this requires (rotates `client_secret`,
        /// drops outpost bindings — can disrupt active sessions).
        DisruptiveProviderKindChangeNotAllowed,
    }
}

impl Reason for ReasonCode {
    const SUCCESS: Self = ReasonCode::Reconciled;

    fn as_str(self) -> &'static str {
        ReasonCode::as_str(self)
    }

    fn severity(self) -> Severity {
        ReasonCode::severity(self)
    }
}

impl ReasonCode {
    pub const fn severity(self) -> Severity {
        match self {
            ReasonCode::NoAccessPolicyBound
            | ReasonCode::Reconciled
            | ReasonCode::Created
            | ReasonCode::Deleted
            | ReasonCode::Released => Severity::Advisory,
            _ => Severity::Blocking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_pascal_case() {
        weebo_kit_domain::reason::assert_pascal_case(ReasonCode::ALL);
        assert_eq!(ReasonCode::ALL.len(), 23);
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
