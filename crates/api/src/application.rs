use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
    #[serde(default)]
    pub allowed_redirect_uris: Vec<RedirectUri>,
    #[serde(default)]
    pub property_mappings: Vec<String>,
    #[serde(default)]
    pub grant_types: Vec<String>,
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
