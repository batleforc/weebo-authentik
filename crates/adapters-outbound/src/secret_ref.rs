//! Reading a [`SecretKeyRef`] — the CRDs' "this value lives in a Kubernetes
//! `Secret`" shape — through the API server.
//!
//! Three call sites want the same read and the same two failure messages:
//! the instance's API token, its `tls.caSecretRef`, and its
//! `secretStore.vault.caSecretRef`. They map the failure into different
//! error enums, so this returns a [`SecretRefError`] each caller wraps in
//! its own variant.

use api::instance::SecretKeyRef;
use kube::Client;

/// Why a [`SecretKeyRef`] could not be read; `what` names the reference.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SecretRefError {
    /// The Secret could not be read (missing, forbidden, API error).
    #[error("{what}: {message}")]
    Read { what: &'static str, message: String },
    #[error("{what} {namespace}/{name} has no key {key:?}")]
    MissingKey {
        what: &'static str,
        namespace: String,
        name: String,
        key: String,
    },
}

/// Returns the raw bytes of `secret_ref`'s key. `what` names the reference's
/// role in the error message ("secret", "Authentik CA secret", "Vault CA
/// secret", ...) so a failure says *which* of an instance's several Secret
/// references is broken.
///
/// A missing Secret or missing key is an error, never an empty/absent value:
/// every caller here is configured-but-unreadable rather than unset (the
/// unset case is an `Option` checked before calling), and silently carrying
/// on would mean running with no token or with the wrong trust roots.
pub(crate) async fn read_secret_key(
    client: &Client,
    secret_ref: &SecretKeyRef,
    what: &'static str,
) -> Result<Vec<u8>, SecretRefError> {
    weebo_kit_adapters::read_secret_key(client, secret_ref)
        .await
        .map_err(|message| SecretRefError::Read { what, message })?
        .ok_or_else(|| SecretRefError::MissingKey {
            what,
            namespace: secret_ref.namespace.clone(),
            name: secret_ref.name.clone(),
            key: secret_ref.key.clone(),
        })
}
