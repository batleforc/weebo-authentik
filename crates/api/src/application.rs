use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;
pub use weebo_kit_api::{InstanceKind, InstanceRef, SecretTarget};

/// Mirrors `authentik_application` + its `protocol_provider`. Namespaced —
/// owned by whichever team/namespace created it, governed by
/// `AuthentikNamespacePolicy`.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikApplication",
    plural = "authentikapplications",
    namespaced,
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikApplicationSpec {
    /// The `AuthentikInstance` or `AuthentikLocalInstance` this application
    /// lives on.
    pub instance_ref: InstanceRef,
    pub name: String,
    /// The application's slug: its URL in Authentik and its identity there
    /// (`status.authentikId`). Changing it renames the application in
    /// place; its access policies follow.
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_icon: Option<String>,
    pub provider: ProviderSpec,
    /// Where this application's oauth2 credentials are written. When empty
    /// (the default), they go to one target named after this CR, in its
    /// namespace, on the `AuthentikInstance`'s `secretStore.backend`. When
    /// non-empty, they are **fanned out** to every listed target;
    /// Kubernetes and Vault targets may be mixed. A target stays in this
    /// CR's namespace, and a Vault `path` under
    /// `<pathPrefix>/<namespace>/` unless the instance's `pathGrants`
    /// open it to this namespace. Only meaningful for
    /// `provider.kind: oauth2` — proxy providers carry no client secret, so
    /// this is ignored for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_targets: Vec<SecretTarget>,
}

/// `oauth2`/`proxy` are implemented in v1 (both are in real use in the
/// Terraform module being replaced — `longhorn.tf` uses `proxy`).
/// `saml`/`ldap` are schema-only stubs: accepted by the schema so a future
/// addition isn't a breaking change, but rejected by the reconciler with
/// an explicit error, never silently ignored. See `.prompt/plan.md`
/// decision 2.
///
/// Deliberately **not** a Rust sum type (`enum Foo { Oauth2(..),
/// Proxy(..) }`) even though that's what it conceptually is: kube-core's
/// CRD schema generation cannot merge an internally-tagged enum whose
/// variants carry different literal discriminator values — each variant's
/// `kind` property gets a different `enum: [...]` schema and
/// `CustomResourceExt::crd()` panics ("Property 'kind' ... must be
/// identical"). This flat-struct-plus-discriminator shape is the standard
/// kube.rs workaround; "exactly one of `oauth2`/`proxy` set, matching
/// `kind`" is validated by the reconciler, not the schema.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpec {
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<Oauth2ProviderSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyProviderSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Oauth2,
    Proxy,
    /// Schema-only stub — no reconciler implements this yet.
    Saml,
    /// Schema-only stub — no reconciler implements this yet.
    Ldap,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Oauth2ProviderSpec {
    /// Public clients cannot safely keep a client secret. Existing CRs that
    /// omit this field retain the historical confidential-client behavior.
    #[serde(default)]
    pub client_type: Oauth2ClientType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    pub authorization_flow: String,
    pub invalidation_flow: String,
    /// Name of an existing Authentik certificate key pair used to sign
    /// tokens. Resolved by name at reconcile time (never created by this
    /// operator). Defaults to Authentik's built-in
    /// `"authentik Self-signed Certificate"` when the field is omitted;
    /// set it explicitly to `null` to create a provider with no signing
    /// key. The default is only applied on *absence* — an explicit `null`
    /// is preserved — so the importer, which emits `null` for a provider
    /// that genuinely has no signing key, keeps migration parity.
    #[serde(default = "default_signing_key")]
    pub signing_key: Option<String>,
    #[serde(default)]
    pub allowed_redirect_uris: Vec<RedirectUri>,
    #[serde(default)]
    pub property_mappings: Vec<String>,
    #[serde(default)]
    pub grant_types: Vec<String>,
    /// How long an access token stays valid, in Authentik's duration
    /// spelling: `"minutes=30"`, `"hours=1"`, `"days=1"`, or several joined
    /// by `;`. It governs the ID token minted alongside it too, which is what
    /// makes it worth setting: a consumer that replays an ID token as a
    /// credential -- `docker login` against a registry that validates OIDC
    /// bearers, for one -- lives exactly this long before it has to get
    /// another.
    ///
    /// Omitted leaves the value alone: the field is not sent, so a provider
    /// tuned by hand keeps its setting and a new one takes Authentik's
    /// default. That is deliberate, and it is what keeps adopting an existing
    /// provider free of surprises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token_validity: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Oauth2ClientType {
    Public,
    #[default]
    Confidential,
}

/// Authentik's built-in self-signed certificate key pair, present on every
/// instance — the sensible default signing key when a provider doesn't
/// name one explicitly. See `.prompt/plan.md` (certificate key pairs are
/// always a lookup of an existing cert, never created by this operator).
fn default_signing_key() -> Option<String> {
    Some("authentik Self-signed Certificate".to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedirectUri {
    pub matching_mode: MatchingMode,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchingMode {
    Strict,
    Regex,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyProviderSpec {
    pub internal_host: String,
    pub external_host: String,
    pub authorization_flow: String,
    pub invalidation_flow: String,
    /// `None` => attach to Authentik's embedded outpost by name (default,
    /// matches current `longhorn.tf` behavior). `Some(name)` => must
    /// resolve to an `AuthentikOutpost` CR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outpost_ref: Option<String>,
}
