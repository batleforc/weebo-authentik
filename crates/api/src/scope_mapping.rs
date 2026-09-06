use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Mirrors `authentik_property_mapping_provider_scope` — the OAuth2/OIDC
/// **scope mapping**, i.e. one entry of the list a provider exposes as
/// `propertyMappings`. Cluster-scoped, like `AuthentikFlow`: a mapping is
/// an org-wide object referenced by name from any number of providers, in
/// any namespace, and is not owned by the namespace of the application
/// that happens to use it first.
///
/// Scope mappings are **pk-keyed** (a UUID), so `status.authentikId`
/// stores that pk, unlike the slug-keyed `AuthentikFlow`/
/// `AuthentikApplication`.
///
/// # Why this kind exists
///
/// `Oauth2ProviderSpec.property_mappings` resolves entries **by name**
/// against `/propertymappings/all/` and never creates one — so until this
/// kind, only Authentik's built-in mappings (`authentik default OAuth
/// Mapping: OpenID 'profile'`, …) could be referenced from a CR. A
/// consumer that needs a claim Authentik does not ship had to have it
/// hand-made in the UI, off the GitOps path.
///
/// The motivating case is RustFS: it reads the `groups` claim as a list of
/// **policy names** and refuses the whole login when any entry does not
/// name a policy it has (`OIDC policy mapping did not resolve to current
/// policies`). Sending it the raw `weebo_*` group names cannot work, so a
/// mapping owned by this kind rewrites them, per-provider, into the policy
/// names that side knows.
///
/// # Ordering
///
/// Nothing here watches `AuthentikApplication`, and an application
/// referencing a mapping by a name that does not exist yet fails its
/// reconcile with `AuthentikApiError` ("property mapping … not found")
/// until the mapping's own reconcile lands. That resolves itself on the
/// application's error requeue; when order matters for a first install,
/// give the mapping a lower Argo CD sync-wave than the application.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikScopeMapping",
    plural = "authentikscopemappings",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikScopeMappingSpec {
    /// Authentik-side name of the mapping, and the identity every
    /// provider uses to reference it (`Oauth2ProviderSpec
    /// .propertyMappings`, matched exactly). Not the CR's own name.
    pub name: String,
    /// OAuth scope the client must request for this mapping to run, e.g.
    /// `groups`. It does **not** have to be unique: a mapping that
    /// overrides a built-in one (say Authentik's own `groups`) simply
    /// reuses that scope name, and the provider decides which of the two
    /// it carries by listing one and not the other in its
    /// `propertyMappings`.
    pub scope_name: String,
    /// The mapping's Python expression, run by Authentik per token issue,
    /// with `request`/`user` in scope. It must `return` a dict, which is
    /// merged into the claims — returning `{"groups": [...]}` under scope
    /// name `groups` replaces the built-in claim of that name.
    ///
    /// Passed through verbatim: this operator does not parse, lint or
    /// sandbox it, and a syntax error surfaces only when Authentik runs
    /// the mapping during a login, not at reconcile time.
    pub expression: String,
    /// Shown to the user on the consent screen. Authentik hides the
    /// mapping from that screen when empty, which is the default here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
