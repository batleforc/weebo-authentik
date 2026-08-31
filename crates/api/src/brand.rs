use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Mirrors `authentik_brand`. Cluster-scoped — a brand is tied to a domain
/// at the instance level, not to an application namespace. See
/// `.prompt/plan.md`, "Election du brand par defaut" for the `default`
/// field's reconcile semantics (implemented in `domain::brand_election`,
/// wired up in the `application` crate's `reconcile_brand` use-case).
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikBrand",
    plural = "authentikbrands",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikBrandSpec {
    pub domain: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding_logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding_favicon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding_default_flow_background: Option<String>,
    /// Name of an `AuthentikApplication` CR, same-namespace convention
    /// does not apply here since this CRD is cluster-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_application_ref: Option<String>,
    /// Flow slugs, resolved at reconcile time. These reference a flow by
    /// slug whether or not an `AuthentikFlow` CR manages it — the
    /// reference is always slug-based, never a Kubernetes CR lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_authentication: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_invalidation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_recovery: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_unenrollment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_user_settings: Option<String>,
}
