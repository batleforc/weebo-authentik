use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use weebo_kit_api::{LocalSecretKeyRef, LocalTlsOptions, SharedWith};

use crate::status::AuthentikStatus;

/// Connection to a real Authentik server, owned by a namespace: its token
/// is that namespace's, and only the `AuthentikApplication`s of this
/// namespace use it unless `sharedWith` says otherwise. What an operator
/// limited to one namespace uses. Credentials of the applications bound to
/// it are written as Kubernetes `Secret`s only: the operator's Vault
/// identity is shared by every namespace, so a namespace's own instance
/// can't route to Vault.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikLocalInstance",
    plural = "authentiklocalinstances",
    namespaced,
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikLocalInstanceSpec {
    /// Base **web** URL of the Authentik instance, e.g.
    /// `https://login.example.com` — not the `/api/v3` REST endpoint, which
    /// is derived from it.
    pub url: String,
    /// The API token the operator authenticates with, in a `Secret` of this
    /// namespace. Read on every reconcile: rotating it needs no restart.
    pub token_secret_ref: LocalSecretKeyRef,
    /// TLS verification for the API connection above; the CA `Secret` lives
    /// in this namespace.
    #[serde(default)]
    pub tls: LocalTlsOptions,
    /// Other namespaces (or cluster-scoped objects) allowed to use this
    /// instance — and its token. Unset: this namespace only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_with: Option<SharedWith>,
}

impl weebo_kit_api::SharesItself for AuthentikLocalInstance {
    fn shared_with(&self) -> Option<&SharedWith> {
        self.spec.shared_with.as_ref()
    }
}
