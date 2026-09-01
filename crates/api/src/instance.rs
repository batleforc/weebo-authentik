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
    /// Base **web** URL of the Authentik instance, e.g.
    /// `https://login.example.com` — not the `/api/v3` REST endpoint, which
    /// is derived from it (see [`split_urls`]). This is the root the
    /// per-application OIDC issuers hang off,
    /// `<url>/application/o/<slug>/`, so it is what oauth2 consumers end up
    /// trusting.
    pub url: String,
    pub token_secret_ref: SecretKeyRef,
    /// TLS verification for the API connection above — a private CA to trust
    /// (`tls.caSecretRef`), or the `tls.insecureSkipVerify` escape hatch.
    /// Defaults to plain platform-trust-store verification.
    #[serde(default)]
    pub tls: TlsOptions,
    /// Where `SecretStore::write_oauth2_credentials`/`delete` writes this
    /// instance's oauth2 application credentials. Defaults to a
    /// Kubernetes `Secret` (`crates/adapters-outbound/src/secret_k8s.rs`)
    /// when unset.
    #[serde(default)]
    pub secret_store: SecretStoreSpec,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub namespace: String,
    pub key: String,
}

/// How the operator verifies the Authentik server's TLS certificate. This
/// covers the **API** connection only — the Vault connection has its own
/// `secretStore.vault.caSecretRef` ([`VaultSecretStoreSpec`]), since the two
/// endpoints are routinely signed by different CAs.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TlsOptions {
    /// Skip certificate verification entirely — insecure, intended only for
    /// throwaway/self-signed instances. Prefer `caSecretRef` below, which
    /// trusts a private CA without turning verification off.
    #[serde(default)]
    pub insecure_skip_verify: bool,
    /// Optional custom CA certificate to verify Authentik's TLS with, read
    /// from a Kubernetes `Secret` (the referenced `key` must hold a PEM
    /// bundle). Use this to trust a private CA without mounting it into the
    /// operator pod — the operator reads the Secret via the API and hands the
    /// PEM to its HTTP client. It is added *on top of* the platform trust
    /// store, so publicly-signed endpoints keep working. When unset, only the
    /// platform trust store (plus the usual `SSL_CERT_FILE`/`SSL_CERT_DIR`
    /// env vars) is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_secret_ref: Option<SecretKeyRef>,
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
    /// Optional custom CA certificate to verify Vault's TLS with, read from
    /// a Kubernetes `Secret` (the referenced `key` must hold a PEM bundle).
    /// Use this to trust a private CA (e.g. openbao-tls) without mounting
    /// it into the operator pod — the operator reads the Secret via the
    /// API and hands the PEM to its Vault client. When unset, the system
    /// trust store is used. Only consulted for `https://` Vault addresses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_secret_ref: Option<SecretKeyRef>,
}

fn default_vault_path_prefix() -> String {
    "weebo-authentik".to_string()
}

fn default_kubernetes_auth_mount() -> String {
    "kubernetes".to_string()
}

/// Derives the two base URLs a client needs from the single [`url`] field
/// this CRD exposes.
///
/// `spec.url` is the instance's **web** base — the root the per-application
/// OIDC issuers hang off, `<web>/application/o/<slug>/`. The generated REST
/// client is the other half: it appends every path to a base that has to end
/// in `/api/v3` (its own `Configuration::default()` base_path is literally
/// `/api/v3`). Passing `spec.url` through for both can only ever satisfy one
/// of them, so the split happens here, once, for every caller.
///
/// A `url` that already carries `/api/v3` is accepted rather than taken
/// literally: the field is documented as the web base, but pasting the API
/// URL is the obvious mistake, and honouring it would write
/// `.../api/v3/application/o/<slug>/` into every oauth2 application's
/// `AUTHENTIK_URL` — an issuer that 404s for every consumer.
///
/// Returns `(api_base_path, web_base_url)`.
///
/// [`url`]: AuthentikInstanceSpec::url
pub fn split_urls(url: &str) -> (String, String) {
    let web = url
        .trim_end_matches('/')
        .trim_end_matches("/api/v3")
        .trim_end_matches('/')
        .to_string();
    let api = format!("{web}/api/v3");
    (api, web)
}

#[cfg(test)]
mod url_tests {
    use super::split_urls;

    #[test]
    fn web_base_gets_the_api_suffix_for_rest_calls() {
        assert_eq!(
            split_urls("https://auth.weebo.poc"),
            (
                "https://auth.weebo.poc/api/v3".to_string(),
                "https://auth.weebo.poc".to_string()
            )
        );
    }

    #[test]
    fn trailing_slash_does_not_double_up() {
        assert_eq!(
            split_urls("https://auth.weebo.poc/"),
            (
                "https://auth.weebo.poc/api/v3".to_string(),
                "https://auth.weebo.poc".to_string()
            )
        );
    }

    #[test]
    fn an_api_url_is_accepted_and_never_reaches_the_issuer() {
        // The field is documented as the web base, but someone pointing it at
        // the REST endpoint must not end up with
        // `.../api/v3/application/o/<slug>/` as an app's AUTHENTIK_URL.
        for url in [
            "https://auth.weebo.poc/api/v3",
            "https://auth.weebo.poc/api/v3/",
        ] {
            assert_eq!(
                split_urls(url),
                (
                    "https://auth.weebo.poc/api/v3".to_string(),
                    "https://auth.weebo.poc".to_string()
                )
            );
        }
    }
}
