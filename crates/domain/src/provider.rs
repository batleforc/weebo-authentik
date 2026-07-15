//! Pure data mirroring `AuthentikApplication.spec.provider`'s `oneOf`.
//! `Oauth2`/`Proxy` are implemented in v1 (both are in real use in the
//! Terraform module being replaced); `Saml`/`Ldap` are schema-only stubs —
//! see `.prompt/plan.md` decision 2.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderKind {
    Oauth2(Oauth2Provider),
    Proxy(ProxyProvider),
    /// Schema-only stub — no reconciler implements this variant yet.
    Saml,
    /// Schema-only stub — no reconciler implements this variant yet.
    Ldap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oauth2Provider {
    pub client_id: Option<String>,
    pub authorization_flow: String,
    pub invalidation_flow: String,
    pub signing_key: Option<String>,
    pub allowed_redirect_uris: Vec<RedirectUri>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectUri {
    pub matching_mode: MatchingMode,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingMode {
    Strict,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyProvider {
    pub internal_host: String,
    pub external_host: String,
    pub authorization_flow: String,
    pub invalidation_flow: String,
    /// `None` => attach to Authentik's embedded outpost by name (the
    /// default, matches current `longhorn.tf` behavior). `Some(name)` =>
    /// must resolve to an `AuthentikOutpost` CR or the reconciler rejects
    /// with `ReasonCode::OutpostRefNotFound`.
    pub outpost_ref: Option<String>,
}
