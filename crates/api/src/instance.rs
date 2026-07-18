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
    /// Where `SecretStore::write_oauth2_credentials`/`delete` writes this
    /// instance's oauth2 application credentials. Defaults to a
    /// Kubernetes `Secret` (`crates/adapters-outbound/src/secret_k8s.rs`)
    /// when unset.
    #[serde(default)]
    pub secret_store: SecretStoreSpec,
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

/// Deliberately **not** a Rust sum type (`enum SecretStoreSpec { Kubernetes,
/// Vault(..) }`), same reason as `ProviderSpec`/`ProviderKind` in
/// `application.rs`: kube-core's CRD schema generation cannot merge an
/// internally-tagged enum whose variants carry different `backend`
/// discriminator schemas. "`vault` set iff `backend: Vault`" is validated
/// by `AuthentikSecretStoreFactory`, not the schema.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretStoreSpec {
    #[serde(default)]
    pub backend: SecretStoreBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<VaultSecretStoreSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SecretStoreBackend {
    #[default]
    Kubernetes,
    Vault,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VaultSecretStoreSpec {
    /// Vault server address, e.g. `https://vault.example.com:8200`.
    pub address: String,
    /// KV v2 secrets engine mount (e.g. `secret`) credentials are written
    /// under.
    pub mount: String,
    /// Path prefix under `mount` — the final KV path is
    /// `<pathPrefix>/<namespace>/<name>`, one secret per
    /// `AuthentikApplication` CR (same convention as the Kubernetes
    /// backend's Secret naming).
    #[serde(default = "default_vault_path_prefix")]
    pub path_prefix: String,
    /// Vault Kubernetes-auth role this operator authenticates as — its
    /// own ServiceAccount JWT is exchanged for a Vault token via
    /// `auth/<kubernetesAuthMount>/login`. See `.prompt/plan.md`'s note
    /// on `vault.tf`'s OIDC federation for why a Vault-aware `SecretStore`
    /// exists at all (unrelated to this auth step, which is the
    /// operator authenticating *to* Vault, not Vault federating
    /// Authentik as an OIDC provider).
    pub kubernetes_auth_role: String,
    /// Mount path of Vault's Kubernetes auth backend. Defaults to
    /// `kubernetes`, Vault's own default mount name.
    #[serde(default = "default_kubernetes_auth_mount")]
    pub kubernetes_auth_mount: String,
}

fn default_vault_path_prefix() -> String {
    "weebo-authentik".to_string()
}

fn default_kubernetes_auth_mount() -> String {
    "kubernetes".to_string()
}
