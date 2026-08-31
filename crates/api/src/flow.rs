use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Mirrors `authentik_flow`. Cluster-scoped — flows are org-wide
/// authentication/authorization pipelines referenced by slug from brands
/// and application providers, not owned by a namespace. Only reachable via
/// cluster-scoped RBAC (no `AuthentikNamespacePolicy` governs this kind).
///
/// Flows are **slug-keyed** in Authentik's API (create returns a `pk`
/// UUID, but update/delete operate on the slug), so `status.authentikId`
/// stores the slug — same convention as `AuthentikApplication`, whose
/// applications API is likewise slug-keyed.
///
/// This CRD manages the flow object itself (name/title/designation/…). Its
/// **stage bindings and policies are out of scope** — the Terraform module
/// being replaced defines a single custom flow (device-code) and otherwise
/// references Authentik's built-in flows, so slug-referencing from
/// `AuthentikBrand`/`AuthentikApplication` remains the norm; this CRD just
/// lets that one custom flow (and any future ones) be declared as a CR
/// instead of assumed to pre-exist.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikFlow",
    plural = "authentikflows",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikFlowSpec {
    /// Visible in the URL and the identity every other CRD uses to
    /// reference this flow (`AuthentikBrand.flow*`,
    /// `Oauth2ProviderSpec.authorizationFlow`, …). Immutable in practice:
    /// changing it creates a new flow rather than renaming the old one.
    pub slug: String,
    /// Internal name of the flow.
    pub name: String,
    /// Title shown to users on the flow's pages.
    pub title: String,
    /// What the flow is used for (authentication, authorization, …).
    pub designation: FlowDesignation,
    /// Required level of authentication to access the flow. Defaults to
    /// Authentik's own default when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<FlowAuthentication>,
    /// How the flow's bound policies are combined. Defaults to Authentik's
    /// own default (`any`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_engine_mode: Option<PolicyEngineMode>,
    /// Increases compatibility with mobile password managers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_mode: Option<bool>,
    /// Visual layout of the flow's pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<FlowLayout>,
    /// What happens when the flow denies access to a user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied_action: Option<FlowDeniedAction>,
    /// Background image shown during flow execution (a URL or a static
    /// path served by Authentik).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

/// Wire values match Authentik's `FlowDesignationEnum`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowDesignation {
    Authentication,
    Authorization,
    Invalidation,
    Enrollment,
    Unenrollment,
    Recovery,
    StageConfiguration,
}

/// Wire values match Authentik's `AuthenticationEnum`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowAuthentication {
    None,
    RequireAuthenticated,
    RequireUnauthenticated,
    RequireSuperuser,
    RequireRedirect,
    RequireOutpost,
    RequireToken,
}

/// Wire values match Authentik's `PolicyEngineMode`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEngineMode {
    All,
    Any,
}

/// Wire values match Authentik's `FlowLayoutEnum`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowLayout {
    Stacked,
    ContentLeft,
    ContentRight,
    SidebarLeft,
    SidebarRight,
    SidebarLeftFrameBackground,
    SidebarRightFrameBackground,
}

/// Wire values match Authentik's `DeniedActionEnum`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowDeniedAction {
    MessageContinue,
    Message,
    Continue,
}
