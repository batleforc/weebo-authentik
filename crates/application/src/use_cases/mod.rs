//! Most `reconcile_*` use-cases below are `async fn` — they call an
//! `AuthentikGateway`/`SecretStoreFactory` over the network. `reconcile_instance`
//! and `reconcile_namespace_policy` are plain sync `fn` instead: both CRDs are
//! purely Kubernetes-side config with no remote Authentik object to reconcile
//! against. Don't assume async across the family — check the specific module.

pub mod evaluate_admission;
pub mod reconcile_access_policy;
pub mod reconcile_application;
pub mod reconcile_brand;
pub mod reconcile_group;
pub mod reconcile_instance;
pub mod reconcile_namespace_policy;
pub mod reconcile_outpost;
pub mod reconcile_user;

use domain::error::ReasonCode;

/// Shared outcome shape for every `reconcile_*` use-case. The controller
/// adapter turns this into a status-condition patch via the `ReasonCode`
/// (never a hand-written string) and, for `Errored`, a Kubernetes `Event`.
/// `authentik_id` is `None` for CRDs with no corresponding remote object
/// (`AuthentikInstance`, `AuthentikNamespacePolicy` — both are purely
/// Kubernetes-side config).
#[derive(Debug)]
pub enum ReconcileOutcome {
    Synced { authentik_id: Option<String> },
    Errored { reason: ReasonCode, message: String },
}

/// Shared `GatewayError` -> `ReconcileOutcome` mapping so every
/// `reconcile_*` use-case surfaces the same `ReasonCode` for the same
/// upstream failure — see `.prompt/plan.md`, "Convention d'erreurs".
pub(crate) fn errored_from_gateway_error(err: crate::ports::GatewayError) -> ReconcileOutcome {
    use crate::ports::GatewayError;
    match err {
        GatewayError::AlreadyExists(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::AuthentikObjectAlreadyExists,
            message,
        },
        GatewayError::NotFound(message) | GatewayError::Api(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::AuthentikApiError,
            message,
        },
    }
}

/// Shared `GatewayFactoryError` -> `ReconcileOutcome` mapping, mirroring
/// `errored_from_gateway_error` above. Public (not `pub(crate)`): unlike
/// the gateway itself, factory resolution happens in `adapters-inbound`
/// controllers, before a `reconcile_*` use-case is ever called — outside
/// this crate.
pub fn errored_from_factory_error(err: crate::ports::GatewayFactoryError) -> ReconcileOutcome {
    use crate::ports::GatewayFactoryError;
    match err {
        GatewayFactoryError::InstanceNotFound(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::InstanceRefNotFound,
            message,
        },
        GatewayFactoryError::AmbiguousDefault(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::AmbiguousDefaultInstance,
            message,
        },
        GatewayFactoryError::ResolutionFailed(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::InstanceResolutionFailed,
            message,
        },
    }
}

/// Shared `SecretStoreFactoryError` -> `ReconcileOutcome` mapping, same
/// rationale as `errored_from_factory_error` above (public: resolution
/// happens in `adapters-inbound` controllers, before
/// `reconcile_application` is ever called).
pub fn errored_from_secret_store_factory_error(
    err: crate::ports::SecretStoreFactoryError,
) -> ReconcileOutcome {
    use crate::ports::SecretStoreFactoryError;
    match err {
        SecretStoreFactoryError::InstanceNotFound(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::InstanceRefNotFound,
            message,
        },
        SecretStoreFactoryError::ResolutionFailed(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::SecretStoreError,
            message,
        },
    }
}
