use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::status::AuthentikStatus;

/// Mirrors `authentik_group`. Cluster-scoped — group hierarchy is an
/// org-wide identity concept, not owned by a namespace. Only reachable via
/// cluster-scoped RBAC (no `AuthentikNamespacePolicy` governs this kind).
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikGroup",
    plural = "authentikgroups",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikGroupSpec {
    pub name: String,
    #[serde(default)]
    pub is_superuser: bool,
    /// Name of another `AuthentikGroup`, for hierarchy (`weebo_user` ->
    /// `weebo_moderator` -> `weebo_admin` in the Terraform module).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}
