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
    /// Parent group, for hierarchy (`weebo_user` -> `weebo_moderator` ->
    /// `weebo_admin` in the Terraform module).
    ///
    /// This is the parent's **Authentik** name — its `spec.name`, the value
    /// Authentik itself stores — not the name of the `AuthentikGroup` CR
    /// describing it. It is resolved with a `/core/groups/?name=` query
    /// against the instance, and no Kubernetes object is read: a CR named
    /// `weebo-user` whose `spec.name` is `weebo_user` must be referenced
    /// here as `weebo_user`. A miss fails the reconcile with
    /// `GroupRefNotFound`, it never creates the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}
