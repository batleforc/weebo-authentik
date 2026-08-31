use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::instance::SecretStoreBackend;
use crate::status::AuthentikStatus;

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
    /// Name of an `AuthentikInstance` CR.
    pub instance_ref: String,
    pub name: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_icon: Option<String>,
    pub provider: ProviderSpec,
    /// Where this application's oauth2 credentials are written. When empty
    /// (the default), the credentials go to the single destination defined
    /// by the `AuthentikInstance`'s `secretStore` (a Kubernetes `Secret`
    /// named after this CR, or the instance's Vault path convention) —
    /// exactly the pre-existing behavior. When non-empty, the credentials
    /// are **fanned out** to every listed target; Kubernetes and Vault
    /// targets may be mixed (e.g. two Vault paths plus one Kubernetes
    /// Secret), and each Vault target may pin an explicit path. Only
    /// meaningful for `provider.kind: oauth2` — proxy providers carry no
    /// client secret, so this is ignored for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_targets: Vec<SecretTarget>,
}

/// One destination the oauth2 credentials are written to when an
/// `AuthentikApplication` opts into explicit `secretTargets` fan-out.
///
/// Vault targets reuse the connection/auth config from the owning
/// `AuthentikInstance`'s `secretStore.vault` (address, mount,
/// Kubernetes-auth role) — an application selects only *where under that
/// mount* to write, never how to reach Vault. A Vault target therefore
/// requires the instance to carry a `secretStore.vault` block even if the
/// instance's default `backend` is `kubernetes`; that mismatch surfaces as
/// a reconcile error, not a silent skip.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretTarget {
    /// Which backend this copy of the credentials is written to.
    pub backend: SecretStoreBackend,
    /// Vault targets only: the exact KV v2 path (under the instance's
    /// `secretStore.vault.mount`) to write to, e.g.
    /// `apps/frontend/oauth`. When unset, the instance's default
    /// `<pathPrefix>/<namespace>/<name>` convention is used. Ignored for
    /// `kubernetes` targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Kubernetes targets only: the name of the `Secret` to write, in this
    /// application's own namespace. When unset, defaults to the
    /// application CR's name (the pre-existing single-destination
    /// convention). Ignored for `vault` targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
