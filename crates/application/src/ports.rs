//! Port traits. Adapters (outbound, in `adapters-outbound`) implement
//! these; use-cases (in this crate) depend only on these traits, never on
//! a concrete adapter — dependency injection happens in `operator::main`.
//! See `.prompt/plan.md`, "Architecture hexagonale".

use std::sync::Arc;

use api::application::{Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec};
use api::{AuthentikApplication, AuthentikBrand, AuthentikGroup, AuthentikOutpost, AuthentikUser};

#[derive(Debug, Clone, thiserror::Error)]
pub enum GatewayError {
    /// Create was rejected because an object with the same name/slug
    /// already exists and isn't tracked by this CR. Maps to
    /// `domain::error::ReasonCode::AuthentikObjectAlreadyExists`.
    #[error("object already exists in Authentik: {0}")]
    AlreadyExists(String),
    #[error("not found in Authentik: {0}")]
    NotFound(String),
    #[error("authentik API error: {0}")]
    Api(String),
}

/// Talks to a single Authentik instance's API. One instantiation per
/// `AuthentikInstance` CR, produced on demand by `GatewayFactory`.
///
/// `#[async_trait]` rather than native `async fn` in trait: this trait is
/// used as `dyn AuthentikGateway` throughout (see `Ctx` and every
/// `reconcile_*` use-case) for the hexagonal dependency-injection pattern
/// — native async-fn-in-traits are not `dyn`-compatible on stable Rust.
#[async_trait::async_trait]
pub trait AuthentikGateway: Send + Sync {
    /// Attempt-create-first, never lookup-then-create (avoids the race
    /// window between concurrent reconciles) — a name/slug collision with
    /// an object this gateway didn't just create must surface as
    /// `GatewayError::AlreadyExists`, never a silent adopt. See
    /// `.prompt/plan.md`, "Modele de status commun".
    ///
    /// `provider_id` is the Authentik-side provider pk, obtained by the
    /// caller from `upsert_oauth2_provider`/`upsert_proxy_provider` —
    /// threaded through explicitly rather than re-derived here, since the
    /// use-case is what decides oauth2 vs proxy.
    async fn create_application(
        &self,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<String, GatewayError>;
    async fn update_application(
        &self,
        authentik_id: &str,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<(), GatewayError>;
    async fn delete_application(&self, authentik_id: &str) -> Result<(), GatewayError>;
    /// The current remote `Application`'s provider — callers needing the
    /// provider id for an update, or its kind (to detect a
    /// `spec.provider.kind` change across reconciles), read it from here
    /// rather than the CR, which never stores it. `authentik_id` is the
    /// application's slug (Authentik's applications API is slug-keyed).
    async fn get_application(&self, authentik_id: &str) -> Result<RemoteApplication, GatewayError>;
    /// Deletes a provider outright by id and kind — used only when
    /// `reconcile_application` detects an authorized `spec.provider.kind`
    /// change (see its module doc): oauth2 and proxy providers have
    /// separate update endpoints, so PATCHing the old id against the new
    /// kind isn't possible, and the old-kind provider is deleted before a
    /// new one of the new kind is created via
    /// `upsert_oauth2_provider`/`upsert_proxy_provider`.
    async fn delete_provider(
        &self,
        authentik_id: i32,
        kind: ProviderKind,
    ) -> Result<(), GatewayError>;

    /// `group.spec.parent_ref`, if set, is resolved to an Authentik group
    /// by matching `name` via the Authentik API — there is no
    /// Kubernetes-side lookup here (the gateway never talks to `kube`),
    /// so this assumes the referenced `AuthentikGroup` CR's `spec.name`
    /// is also its Authentik-side name, which is the convention this
    /// operator establishes. Resolution failure surfaces as
    /// `GatewayError::NotFound`.
    async fn create_group(&self, group: &AuthentikGroup) -> Result<String, GatewayError>;
    async fn update_group(
        &self,
        authentik_id: &str,
        group: &AuthentikGroup,
    ) -> Result<(), GatewayError>;
    async fn delete_group(&self, authentik_id: &str) -> Result<(), GatewayError>;

    /// `user.spec.group_refs` resolved the same way as
    /// `AuthentikGroupSpec.parent_ref` above (by Authentik-side group
    /// name).
    async fn create_user(&self, user: &AuthentikUser) -> Result<String, GatewayError>;
    async fn update_user(
        &self,
        authentik_id: &str,
        user: &AuthentikUser,
    ) -> Result<(), GatewayError>;
    async fn delete_user(&self, authentik_id: &str) -> Result<(), GatewayError>;

    async fn create_outpost(&self, outpost: &AuthentikOutpost) -> Result<String, GatewayError>;
    async fn update_outpost(
        &self,
        authentik_id: &str,
        outpost: &AuthentikOutpost,
    ) -> Result<(), GatewayError>;
    async fn delete_outpost(&self, authentik_id: &str) -> Result<(), GatewayError>;
    /// `outpost_ref` is `AuthentikApplication.spec.provider.proxy.outpostRef`
    /// verbatim: `None` resolves to Authentik's embedded outpost by its
    /// well-known name (matches `longhorn.tf`'s current behavior); `Some`
    /// resolves by matching an `AuthentikOutpost`'s `name` — not by
    /// Kubernetes CR lookup, same caveat as `create_group`. Unresolvable
    /// maps to `ReasonCode::OutpostRefNotFound` at the use-case layer.
    async fn attach_outpost(
        &self,
        outpost_ref: Option<&str>,
        provider_authentik_id: &str,
    ) -> Result<(), GatewayError>;

    /// Domain/branding/flow fields only — the `default` toggle is a
    /// separate call (`set_brand_default`) since it's driven by
    /// Kubernetes-side election, not the CR's own patch-first sync. See
    /// `.prompt/plan.md`, "Election du brand par defaut".
    ///
    /// `spec.default_application_ref` is deliberately **not** resolved
    /// here: it names an `AuthentikApplication` CR, but `AuthentikBrand`
    /// is cluster-scoped with no namespace field to search in, and
    /// `.prompt/plan.md` never settles that ambiguity (unlike
    /// `AuthentikAccessPolicy.applicationRef`, which is pinned to the
    /// policy's own namespace). Left unset until that's decided, rather
    /// than guessing a cross-namespace search convention no namespace
    /// convention actually authorizes.
    async fn create_brand(&self, brand: &AuthentikBrand) -> Result<String, GatewayError>;
    async fn update_brand(
        &self,
        authentik_id: &str,
        brand: &AuthentikBrand,
    ) -> Result<(), GatewayError>;
    async fn delete_brand(&self, authentik_id: &str) -> Result<(), GatewayError>;

    /// `None` when no brand is currently marked default.
    async fn get_default_brand(&self) -> Result<Option<String>, GatewayError>;
    async fn set_brand_default(
        &self,
        authentik_id: &str,
        default: bool,
    ) -> Result<(), GatewayError>;

    /// oauth2/proxy-specific create/update, kept separate from
    /// `create_application`/`update_application` since the provider
    /// payload shape differs per `ProviderSpec` variant. `authentik_id`
    /// (the provider's pk) is `None` on first reconcile, `Some` to patch
    /// in place — the use-case decides which by reading the current
    /// `provider` FK off `get_application`. `name` is the Authentik-side
    /// provider name, which `ProviderSpec` itself carries no field for
    /// (only the owning `AuthentikApplication` has a name/slug) — the
    /// use-case passes the application's name through.
    async fn upsert_oauth2_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &Oauth2ProviderSpec,
    ) -> Result<Oauth2ProviderUpsertResult, GatewayError>;
    async fn upsert_proxy_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &ProxyProviderSpec,
    ) -> Result<String, GatewayError>;

    /// Mirrors `authentik_policy_binding`. `application_authentik_id` is
    /// the application's slug (as stored in `AuthentikApplication.status`);
    /// `group_name` is resolved the same way as `AuthentikGroup.parent_ref`.
    async fn upsert_policy_binding(
        &self,
        authentik_id: Option<&str>,
        application_authentik_id: &str,
        group_name: &str,
        order: i32,
        negate: bool,
    ) -> Result<String, GatewayError>;
    async fn delete_policy_binding(&self, authentik_id: &str) -> Result<(), GatewayError>;
}

/// The subset of Authentik's remote `Application` object callers need —
/// adapters translate their own wire format into this at the port
/// boundary, same discipline as every other typed method here.
#[derive(Debug, Clone, Default)]
pub struct RemoteApplication {
    /// The currently attached provider's pk, if any.
    pub provider_id: Option<i32>,
    /// Authentik's internal model name for the current provider (e.g.
    /// `"authentik_providers_oauth2.oauth2provider"`) — used by
    /// `reconcile_application` to detect a `spec.provider.kind` change.
    /// `None` alongside a `provider_id` is treated the same as no
    /// provider attached at all (shouldn't happen in practice: Authentik
    /// returns `provider_obj` whenever `provider` is set).
    pub provider_meta_model_name: Option<String>,
}

/// Result of upserting an oauth2 provider: the pk (as `authentik_id`
/// elsewhere) plus the credentials `reconcile_application` must hand to
/// `SecretStore` — proxy providers have no equivalent since auth happens
/// at the outpost, hence this type is oauth2-only rather than a field on
/// a shared result shape.
#[derive(Debug, Clone)]
pub struct Oauth2ProviderUpsertResult {
    pub authentik_id: String,
    pub credentials: Oauth2Credentials,
}

#[derive(Debug, Clone)]
pub struct Oauth2Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub authentik_url: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretStoreError {
    #[error("secret store write failed: {0}")]
    Write(String),
    #[error("secret store delete failed: {0}")]
    Delete(String),
}

/// Canonical secret shape: `AUTHENTIK_CLIENT_ID`/`AUTHENTIK_CLIENT_SECRET`/
/// `AUTHENTIK_URL`, matching every oauth2 app in the Terraform module being
/// replaced. Never called for proxy providers — they carry no client
/// secret (auth happens at the outpost). See `.prompt/plan.md`.
///
/// `#[async_trait]` for the same `dyn`-compatibility reason as
/// `AuthentikGateway`.
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    async fn write_oauth2_credentials(
        &self,
        namespace: &str,
        name: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError>;
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretStoreFactoryError {
    /// `instanceRef` does not resolve to an `AuthentikInstance` CR. Maps
    /// to `domain::error::ReasonCode::InstanceRefNotFound` — same failure
    /// mode as `GatewayFactoryError::InstanceNotFound`.
    #[error("AuthentikInstance {0:?} not found")]
    InstanceNotFound(String),
    /// Kubernetes-side failure resolving the `AuthentikInstance`, or a
    /// failure building/authenticating the chosen `SecretStore` backend
    /// (e.g. Vault Kubernetes-auth login failed, KV mount misconfigured).
    /// Maps to `domain::error::ReasonCode::SecretStoreError`.
    #[error("failed resolving SecretStore backend: {0}")]
    ResolutionFailed(String),
}

/// Produces a `SecretStore` for a given `AuthentikInstance` CR from its
/// `spec.secretStore`. Only `AuthentikApplication` ever needs a
/// `SecretStore` (oauth2 credentials), and it always carries an explicit
/// `instanceRef`, so unlike `GatewayFactory` there is no `default_store`.
///
/// The concrete adapter (`AuthentikSecretStoreFactory`) caches the Vault
/// backend's login for its lease lifetime — each `login` mints a throwaway
/// Vault token, so re-authenticating per reconcile churned tokens
/// needlessly; a KV op rejected as unauthorized self-heals by re-logging-in
/// (see `secret_vault.rs`). The Kubernetes backend is a zero-cost wrapper
/// and is not cached. This is an adapter-side concern the port stays
/// agnostic to.
///
/// `#[async_trait]` for the same `dyn`-compatibility reason as
/// `AuthentikGateway`.
#[async_trait::async_trait]
pub trait SecretStoreFactory: Send + Sync {
    async fn secret_store_for(
        &self,
        instance_ref: &str,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayFactoryError {
    /// `instanceRef` does not resolve to an `AuthentikInstance` CR. Maps
    /// to `domain::error::ReasonCode::InstanceRefNotFound`.
    #[error("AuthentikInstance {0:?} not found")]
    InstanceNotFound(String),
    /// A CRD with no `instanceRef` field asked for the single default
    /// instance, but the cluster has zero or more than one
    /// `AuthentikInstance` CR. Maps to
    /// `domain::error::ReasonCode::AmbiguousDefaultInstance`.
    #[error("ambiguous default AuthentikInstance: {0}")]
    AmbiguousDefault(String),
    /// Kubernetes-side failure resolving the `AuthentikInstance` or its
    /// token `Secret` (API error, missing secret, missing/non-utf8 key).
    /// Maps to `domain::error::ReasonCode::InstanceResolutionFailed`.
    #[error("failed resolving AuthentikInstance: {0}")]
    ResolutionFailed(String),
}

/// Produces an `AuthentikGateway` for a given `AuthentikInstance` CR.
/// The concrete adapter (`AuthentikGatewayFactory`) resolves the
/// `AuthentikInstance` through a shared reflector `Store` (live-updated, so
/// config edits are reflected without a per-call apiserver read) and still
/// reads the `tokenSecretRef` token fresh on every call so rotation is
/// picked up immediately. See `.prompt/plan.md`;
/// `AuthentikApplication.spec.instanceRef` is the only CRD field that names
/// an instance explicitly today, so every other CRD
/// (`AuthentikGroup`/`AuthentikUser`/`AuthentikOutpost`/`AuthentikBrand`)
/// goes through `default_gateway` instead.
///
/// `#[async_trait]` for the same `dyn`-compatibility reason as
/// `AuthentikGateway`.
#[async_trait::async_trait]
pub trait GatewayFactory: Send + Sync {
    async fn gateway_for(
        &self,
        instance_ref: &str,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError>;
    /// Used by CRDs with no `instanceRef` field: resolves to the single
    /// `AuthentikInstance` CR in the cluster, or `AmbiguousDefault` if
    /// there is zero or more than one.
    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError>;
}
