use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Connection to a real Authentik server. Cluster-scoped — an org-wide,
/// shared piece of infrastructure, not owned by a single namespace.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikInstance",
    plural = "authentikinstances",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikInstanceSpec {
    /// Base URL of the Authentik instance, e.g. `https://login.example.com`.
    pub url: String,
    pub token_secret_ref: SecretKeyRef,
    #[serde(default)]
    pub tls: TlsOptions,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub namespace: String,
    pub key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TlsOptions {
    #[serde(default)]
    pub insecure_skip_verify: bool,
}
