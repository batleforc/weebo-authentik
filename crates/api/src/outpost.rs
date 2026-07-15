use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Deliberately minimal — nothing in the Terraform module being replaced
/// creates a custom outpost, everything attaches to Authentik's embedded
/// outpost by default. This CRD exists only to cover the case where a
/// proxy provider's `outpostRef` explicitly names one. Cluster-scoped.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikOutpost",
    plural = "authentikoutposts",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikOutpostSpec {
    pub name: String,
    pub r#type: OutpostType,
    /// Passthrough to the Authentik API — deliberately not modeled field by
    /// field. `x-kubernetes-preserve-unknown-fields` is required here: a
    /// bare `serde_json::Value` schema has no `type`, which a real
    /// structural-schema apiserver rejects outright ("must not be empty
    /// for specified object fields") — caught by the layer-3 envtest
    /// integration test, not by `cargo check`.
    #[serde(default)]
    #[schemars(extend("type" = "object", "x-kubernetes-preserve-unknown-fields" = true))]
    pub config: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutpostType {
    /// The only variant with real usage today.
    Proxy,
    Ldap,
    Radius,
}
