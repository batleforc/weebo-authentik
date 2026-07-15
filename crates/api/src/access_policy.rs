use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Mirrors `authentik_policy_binding`. Namespaced.
///
/// `application_ref` is deliberately a bare name with **no namespace
/// field** — there is no way to express a cross-namespace reference in
/// this schema. The reconciler must resolve it via
/// `Api::<AuthentikApplication>::namespaced(policy.metadata.namespace)`,
/// never a cluster-wide client. See `.prompt/plan.md`,
/// "Confinement au namespace, deux couches".
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikAccessPolicy",
    plural = "authentikaccesspolicies",
    namespaced,
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikAccessPolicySpec {
    /// Name of an `AuthentikApplication` CR in the same namespace.
    pub application_ref: String,
    /// Name of an `AuthentikGroup` CR (cluster-scoped).
    pub group_ref: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub negate: bool,
}
