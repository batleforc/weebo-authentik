//! Reading a [`SecretKeyRef`] — the CRDs' "this value lives in a Kubernetes
//! `Secret`" shape — through the API server.
//!
//! Three call sites want the same read and the same two failure messages:
//! the instance's API token, its `tls.caSecretRef`, and its
//! `secretStore.vault.caSecretRef`. They map the failure into different
//! error enums, so this returns a plain `String` each caller wraps in its
//! own variant.

use api::instance::SecretKeyRef;
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::Api;

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
    what: &str,
) -> Result<Vec<u8>, String> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &secret_ref.namespace);
    let secret = secrets.get(&secret_ref.name).await.map_err(|e| {
        format!(
            "fetching {what} {}/{}: {e}",
            secret_ref.namespace, secret_ref.name
        )
    })?;
    secret
        .data
        .as_ref()
        .and_then(|data| data.get(&secret_ref.key))
        .map(|bytes| bytes.0.clone())
        .ok_or_else(|| {
            format!(
                "{what} {}/{} has no key {:?}",
                secret_ref.namespace, secret_ref.name, secret_ref.key
            )
        })
}
