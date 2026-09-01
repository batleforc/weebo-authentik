use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Mirrors `authentik_user`. Cluster-scoped, like `AuthentikGroup`.
///
/// Deliberately has **no credential/password field** — Authentik owns its
/// own invite/reset flows. If a bootstrap password is ever needed it goes
/// through `SecretStore` (generated, never stored in this spec). See
/// `.prompt/plan.md`.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikUser",
    plural = "authentikusers",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikUserSpec {
    pub username: String,
    pub name: String,
    pub email: String,
    #[serde(default = "default_true")]
    pub is_active: bool,
    /// Groups this user belongs to, by their **Authentik** name
    /// (`AuthentikGroup.spec.name`), not the name of the CR describing them
    /// — each is resolved with a `/core/groups/?name=` query, never read
    /// from Kubernetes. An unresolvable entry fails the reconcile with
    /// `GroupRefNotFound`.
    #[serde(default)]
    pub group_refs: Vec<String>,
}

fn default_true() -> bool {
    true
}
