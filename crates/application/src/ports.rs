//! Port traits. Adapters (outbound, in `adapters-outbound`) implement
//! these; use-cases (in this crate) depend only on these traits, never on
//! a concrete adapter — dependency injection happens in `operator::main`.
//! See `.prompt/plan.md`, "Architecture hexagonale".

use std::collections::BTreeMap;
use std::sync::Arc;

use api::application::{Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec};
use api::{
    AuthentikApplication, AuthentikBrand, AuthentikFlow, AuthentikGroup, AuthentikOutpost,
    AuthentikScopeMapping, AuthentikUser,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum GatewayError {
    /// Create was rejected because an object with the same name/slug
    /// already exists and isn't tracked by this CR. Maps to
    /// `domain::error::ReasonCode::AuthentikObjectAlreadyExists`.
    #[error("object already exists in Authentik: {0}")]
    AlreadyExists(String),
    /// Something the call *references* (a parent group, a flow, an
    /// outpost…) doesn't resolve — never the object addressed by the
    /// call's own `authentik_id`, which is [`GatewayError::Gone`].
    #[error("not found in Authentik: {0}")]
    NotFound(String),
    /// The object addressed by the recorded `authentik_id` no longer
    /// exists (deleted out of band): the CR is authoritative, so an update
    /// recreates it and a cleanup counts it as already deleted.
    #[error("no longer in Authentik: {0}")]
    Gone(String),
    #[error("authentik API error: {0}")]
    Api(String),
}

pub use domain::drift::RemoteWrite;

/// A write the gateway made to Authentik: an object created, put back
/// after a drift (`fields`: the ones corrected), or deleted. Read by the
/// controller after the reconcile ([`AuthentikGateway::take_writes`]) for
/// the `remote_writes_total` counter and the `DriftCorrected` Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The Authentik object (`group`, `oauth2_provider`…).
    pub object: &'static str,
    pub cause: RemoteWrite,
    pub fields: Vec<&'static str>,
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
    /// The writes made since the last call, oldest first. A gateway is
    /// built for each reconcile, so these are that reconcile's. Test
    /// doubles may keep the default: no write reported.
    fn take_writes(&self) -> Vec<Written> {
        Vec::new()
    }

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
    /// Deletes a provider by id whatever its kind (Authentik's
    /// `/providers/all/` endpoint): the provider recorded in an
    /// application's status may be of a kind its spec no longer names.
    /// Already deleted is success.
    async fn delete_any_provider(&self, authentik_id: i32) -> Result<(), GatewayError>;

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

    /// Flows are slug-keyed in Authentik's API, so `authentik_id` here is
    /// the flow's slug (as stored in `AuthentikFlow.status.authentikId`),
    /// not a numeric/UUID pk — same convention as `create_application`.
    /// A flow references no other CRD, so there is no name-resolution step
    /// and thus no `GatewayError::NotFound`-as-unresolved-ref path.
    async fn create_flow(&self, flow: &AuthentikFlow) -> Result<String, GatewayError>;
    async fn update_flow(
        &self,
        authentik_id: &str,
        flow: &AuthentikFlow,
    ) -> Result<(), GatewayError>;
    async fn delete_flow(&self, authentik_id: &str) -> Result<(), GatewayError>;

    /// Scope mappings are pk-keyed (a UUID), so `authentik_id` here is
    /// that pk — not a name, even though a *provider* references the same
    /// object by name through `resolve_property_mappings`. The two never
    /// meet: this operator creates the mapping and remembers its pk, while
    /// `Oauth2ProviderSpec.property_mappings` looks the same object up by
    /// name at provider-upsert time, exactly as it does for the built-in
    /// mappings it cannot create.
    ///
    /// The expression is sent verbatim; Authentik accepts any syntactically
    /// valid Python here and only fails when it runs the mapping, so a
    /// `Synced` status on this kind says the object exists, not that it
    /// produces the claim its author intended.
    async fn create_scope_mapping(
        &self,
        mapping: &AuthentikScopeMapping,
    ) -> Result<String, GatewayError>;
    async fn update_scope_mapping(
        &self,
        authentik_id: &str,
        mapping: &AuthentikScopeMapping,
    ) -> Result<(), GatewayError>;
    async fn delete_scope_mapping(&self, authentik_id: &str) -> Result<(), GatewayError>;

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
    /// `provider` FK off `get_application` (or, the application being
    /// gone, the provider recorded in the status). Patching a provider
    /// that no longer exists is [`GatewayError::Gone`]. `name` is the Authentik-side
    /// provider name, which `ProviderSpec` itself carries no field for
    /// (only the owning `AuthentikApplication` has a name/slug) — the
    /// use-case passes the application's name through.
    /// `slug` is the application's slug, needed to build the per-application
    /// OIDC issuer (`<web_base>/application/o/<slug>/`) written into the
    /// returned `Oauth2Credentials.authentik_url` — the value every
    /// downstream consumer uses as its `AUTHENTIK_URL`/issuer.
    async fn upsert_oauth2_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        slug: &str,
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
/// elsewhere) plus the credentials `reconcile_application` writes to the
/// application's secret targets — proxy providers have no equivalent since auth happens
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

impl Oauth2Credentials {
    /// Canonical secret shape: `AUTHENTIK_CLIENT_ID`/`AUTHENTIK_CLIENT_SECRET`/
    /// `AUTHENTIK_URL`, matching every oauth2 app in the Terraform module
    /// being replaced. A public client has no secret: the key is left out.
    pub fn to_secret_data(&self) -> BTreeMap<String, String> {
        let mut data = BTreeMap::from([
            ("AUTHENTIK_CLIENT_ID".to_string(), self.client_id.clone()),
            ("AUTHENTIK_URL".to_string(), self.authentik_url.clone()),
        ]);
        if !self.client_secret.is_empty() {
            data.insert(
                "AUTHENTIK_CLIENT_SECRET".to_string(),
                self.client_secret.clone(),
            );
        }
        data
    }
}

/// Where emitted credentials are written: the kit's ports. A store routes
/// each `SecretTarget` to its backend, records the owning CR on every value
/// and refuses to overwrite or delete another owner's.
pub use weebo_kit_application::ports::{
    SecretOwner, SecretStore, SecretStoreError, SecretStoreFactory,
};

/// The `AuthentikNamespacePolicy` rules, one list per policy (an empty
/// list: no policy exists, everything is allowed) — read by the admission
/// webhook and re-checked by the reconcilers: the kit's port, as
/// `PolicySource<api::namespace_policy::NamespaceRule>`.
pub use weebo_kit_application::ports::{ClusterError, PolicySource};

/// Which instance a CR lives on — an `AuthentikInstance` or an
/// `AuthentikLocalInstance` — resolved by the kit's rules
/// (`weebo_kit_application::instances`).
pub use weebo_kit_application::instances::{
    BoundInstance, InstanceDirectory, InstanceError, ResolveError,
};

/// The Vault connection of a resolved instance, as the admission webhook
/// checks a CR's Vault targets against it (`secretStore.vault`: its
/// `pathPrefix` and `pathGrants`). Read from the binary's instance
/// reflectors: no API call per request.
#[async_trait::async_trait]
pub trait InstanceVaults: Send + Sync {
    /// `Ok(None)`: the instance is gone. `Ok(Some(None))`: it has no Vault
    /// connection — always the case of an `AuthentikLocalInstance`.
    async fn vault_of(
        &self,
        instance: &BoundInstance,
    ) -> Result<Option<Option<api::instance::VaultSecretStoreSpec>>, String>;
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayFactoryError {
    /// The instance resolved for a CR is gone. Maps to
    /// `domain::error::ReasonCode::InstanceRefNotFound`.
    #[error("{0} not found")]
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
    /// An `AuthentikLocalInstance`'s token `Secret` lacks the consent label
    /// (`authentik.weebo.io/instance-token: "true"`). Maps to
    /// `domain::error::ReasonCode::InstanceTokenNotAllowed`.
    #[error("{0}")]
    TokenNotAllowed(String),
}

/// Produces an `AuthentikGateway` for a resolved instance — an
/// `AuthentikInstance` or an `AuthentikLocalInstance` (see
/// [`InstanceDirectory`]). The concrete adapter (`AuthentikGatewayFactory`)
/// reads the instance from a shared reflector `Store` and the
/// `tokenSecretRef` token fresh on every call, so rotation is picked up
/// immediately. The cluster-scoped kinds, which have no `instanceRef`,
/// go through `default_gateway` (the only `AuthentikInstance`).
///
/// `#[async_trait]` for the same `dyn`-compatibility reason as
/// `AuthentikGateway`.
#[async_trait::async_trait]
pub trait GatewayFactory: Send + Sync {
    async fn gateway_for(
        &self,
        instance: &BoundInstance,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError>;
    /// Used by CRDs with no `instanceRef` field: resolves to the single
    /// `AuthentikInstance` CR in the cluster, or `AmbiguousDefault` if
    /// there is zero or more than one.
    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError>;
}
