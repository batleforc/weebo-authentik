use std::time::Duration;

use api::instance::VaultSecretStoreSpec;
use application::ports::{Oauth2Credentials, SecretStore, SecretStoreError};
use tokio::sync::RwLock;
use vaultrs::client::{Client as _, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;
use vaultrs::{auth::kubernetes, kv2};

/// KV v2 mount/path convention (previously open in `.prompt/plan.md`):
/// one secret per `AuthentikApplication` CR at
/// `<mount>/data/<path_prefix>/<namespace>/<name>`, mirroring
/// `K8sSecretStore`'s one-Secret-per-CR, same-name-same-namespace
/// convention.
///
/// Built per `AuthentikInstance` by `AuthentikSecretStoreFactory`, which
/// **caches** the built store for the lifetime of its Vault login lease
/// (see `cache_ttl`) rather than re-authenticating on every reconcile —
/// each `kubernetes::login` mints a fresh Vault token that lingers for its
/// whole TTL, so re-logging-in per reconcile churned Vault tokens
/// needlessly. The token held here is refreshed in place, without
/// rebuilding the store, whenever a KV op is rejected as
/// unauthenticated/unauthorized (see `relogin_on_auth_error`).
pub struct VaultSecretStore {
    /// Interior-mutable so an auth-error self-heal can swap in a freshly
    /// logged-in token (`set_token`) without the factory rebuilding the
    /// whole store. Read-locked for KV ops, write-locked only to re-login.
    client: RwLock<VaultClient>,
    mount: String,
    path_prefix: String,
    /// Retained so the store can re-login itself on an auth error — the
    /// address already lives in `client`, but the Kubernetes-auth
    /// mount/role and this pod's JWT are needed to repeat the login.
    kubernetes_auth_mount: String,
    kubernetes_auth_role: String,
    jwt: String,
    /// How long the factory may reuse this store before rebuilding it with
    /// a fresh login — 80% of the login lease, or `None` when the token is
    /// not safely cacheable (a zero/omitted lease or a non-renewable
    /// token), in which case the factory re-logs-in on every call.
    cache_ttl: Option<Duration>,
}

impl VaultSecretStore {
    /// Authenticates against Vault's Kubernetes auth backend
    /// (`auth/<kubernetes_auth_mount>/login`) using the given ServiceAccount
    /// JWT, then builds a `VaultClient` carrying the resulting client token
    /// — no static `VAULT_TOKEN` to manage/rotate. `jwt` is read by the
    /// caller (`AuthentikSecretStoreFactory`, from this pod's own projected
    /// token) rather than by this method, so the Vault login flow itself
    /// stays testable against a fake JWT without touching a real
    /// filesystem path.
    /// `ca_pem`, when `Some`, is a PEM bundle (read by the caller from the
    /// `spec.ca_secret_ref` Secret) used to verify Vault's TLS. `vaultrs`
    /// only accepts CA certs as *file paths* (it `std::fs::read`s them while
    /// building the client), so the bytes are materialized to a
    /// short-lived temp file that exists only across `VaultClient::new` —
    /// reqwest loads the cert into memory there, after which the file is
    /// removed. This keeps the private CA out of any mounted volume (the
    /// operator chart has no `extraVolumes`), reading it via the API
    /// instead.
    pub async fn new(
        spec: &VaultSecretStoreSpec,
        jwt: &str,
        ca_pem: Option<&[u8]>,
    ) -> Result<Self, SecretStoreError> {
        let mut builder = VaultClientSettingsBuilder::default();
        builder.address(&spec.address);

        // Kept alive until after `VaultClient::new` reads it; dropped (and
        // the file removed) at end of scope.
        let _ca_file = match ca_pem {
            Some(pem) => {
                let guard = TempCaFile::write(pem)?;
                builder.ca_certs(vec![guard.path_string()]);
                Some(guard)
            }
            None => None,
        };

        let settings = builder
            .build()
            .map_err(|e| SecretStoreError::Write(format!("building Vault client settings: {e}")))?;
        let mut client = VaultClient::new(settings)
            .map_err(|e| SecretStoreError::Write(format!("building Vault client: {e}")))?;

        let auth_info = kubernetes::login(
            &client,
            &spec.kubernetes_auth_mount,
            &spec.kubernetes_auth_role,
            jwt.trim(),
        )
        .await
        .map_err(|e| {
            SecretStoreError::Write(format!(
                "Vault Kubernetes auth login (mount {:?}, role {:?}) failed: {e}",
                spec.kubernetes_auth_mount, spec.kubernetes_auth_role
            ))
        })?;
        client.set_token(&auth_info.client_token);

        Ok(Self {
            client: RwLock::new(client),
            mount: spec.mount.clone(),
            path_prefix: spec.path_prefix.clone(),
            kubernetes_auth_mount: spec.kubernetes_auth_mount.clone(),
            kubernetes_auth_role: spec.kubernetes_auth_role.clone(),
            jwt: jwt.trim().to_string(),
            cache_ttl: cache_ttl_from_lease(auth_info.lease_duration, auth_info.renewable),
        })
    }

    /// How long `AuthentikSecretStoreFactory` may reuse this store before
    /// rebuilding it — `None` means "do not cache" (re-login every call).
    pub fn cache_ttl(&self) -> Option<Duration> {
        self.cache_ttl
    }

    /// The default KV path for a CR when no explicit `secretTargets.path`
    /// is given: `<pathPrefix>/<namespace>/<name>`, one secret per
    /// `AuthentikApplication` CR (same convention as the Kubernetes
    /// backend's Secret naming). Public so `FanOutSecretStore` can derive
    /// it for a Vault target that opted for the convention rather than an
    /// explicit path.
    pub fn default_path(&self, namespace: &str, name: &str) -> String {
        format!("{}/{namespace}/{name}", self.path_prefix)
    }

    /// Re-authenticates in place and swaps the fresh token onto the
    /// existing client — called when a KV op comes back
    /// unauthenticated/unauthorized (an out-of-band token revocation, or a
    /// token that lapsed before the factory's coarse `cache_ttl` window
    /// elapsed). The write lock serializes this against concurrent KV ops.
    async fn relogin(&self) -> Result<(), SecretStoreError> {
        let mut client = self.client.write().await;
        let auth_info = kubernetes::login(
            &*client,
            &self.kubernetes_auth_mount,
            &self.kubernetes_auth_role,
            &self.jwt,
        )
        .await
        .map_err(|e| {
            SecretStoreError::Write(format!(
                "Vault Kubernetes auth re-login (mount {:?}, role {:?}) failed: {e}",
                self.kubernetes_auth_mount, self.kubernetes_auth_role
            ))
        })?;
        client.set_token(&auth_info.client_token);
        Ok(())
    }
}

/// 80% of the login lease, leaving headroom to refresh before the token
/// actually expires. A zero lease or a non-renewable token is treated as
/// non-cacheable so the factory never pins a token that could already be
/// dead.
fn cache_ttl_from_lease(lease_duration: u64, renewable: bool) -> Option<Duration> {
    if lease_duration == 0 || !renewable {
        return None;
    }
    Some(Duration::from_secs(lease_duration * 4 / 5))
}

/// A CA PEM bundle materialized to a temp file for the lifetime of a
/// single `VaultClient::new` call (see `VaultSecretStore::new`), removed on
/// drop. `vaultrs` reads CA certs from paths, not memory, so this bridges a
/// Secret-sourced PEM to that file-path API without a mounted volume.
struct TempCaFile {
    path: std::path::PathBuf,
}

impl TempCaFile {
    fn write(pem: &[u8]) -> Result<Self, SecretStoreError> {
        // A unique name avoids collisions between concurrently-built Vault
        // connections for different instances.
        let path = std::env::temp_dir().join(format!(
            "weebo-authentik-vault-ca-{}.pem",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, pem).map_err(|e| {
            SecretStoreError::Write(format!(
                "materializing Vault CA cert to {}: {e}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }

    fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempCaFile {
    fn drop(&mut self) {
        // Best-effort: the cert is already loaded into the reqwest client by
        // now, so a failed unlink only leaves a temp file, never breaks TLS.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A Vault response that means "your token is no good" — 403 (permission
/// denied, Vault's response for an expired/revoked token) or 401. Anything
/// else is a genuine error the caller must see, not retry.
fn is_auth_error(err: &ClientError) -> bool {
    matches!(err, ClientError::APIError { code, .. } if *code == 401 || *code == 403)
}

impl VaultSecretStore {
    /// Writes the canonical oauth2 credential shape to an **explicit** KV
    /// path under `self.mount` (the caller has already resolved the path —
    /// either an application's `secretTargets.path` or `default_path`). The
    /// `SecretStore::write_oauth2_credentials` impl and `FanOutSecretStore`
    /// both funnel through here so the auth self-heal lives in one place.
    pub async fn write_path(
        &self,
        path: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        let data = serde_json::json!({
            "AUTHENTIK_CLIENT_ID": credentials.client_id,
            "AUTHENTIK_CLIENT_SECRET": credentials.client_secret,
            "AUTHENTIK_URL": credentials.authentik_url,
        });

        // KV v2 `set` is not idempotent: it creates a new version even when
        // the document is byte-identical. The application reconciler rewrites
        // credentials on *every* pass, and a `Synced` outcome requeues after
        // 300s (`controller::requeue_after`), so writing unconditionally
        // churned one version per application per five minutes — ~288/day per
        // path, which rolls the default 10-version history over in well under
        // an hour and puts a permanent write load on Vault for no change.
        //
        // `K8sSecretStore` has no such problem — re-applying an identical
        // Secret is a genuine no-op at the API server — which is why this only
        // ever bit the Vault backend, and why the fix belongs here rather than
        // in the reconciler that both backends share.
        //
        // Drift is still corrected: a hand-edited path no longer matches and
        // is rewritten on the next pass.
        if self.path_matches(path, &data).await {
            return Ok(());
        }

        // The read guard must be released before `relogin` (which takes the
        // write lock), so the KV call is scoped to its own block rather
        // than left as a `match` scrutinee temporary — those live until the
        // end of the match and would deadlock the re-login.
        let first = {
            let client = self.client.read().await;
            kv2::set(&*client, &self.mount, path, &data).await
        };
        match first {
            Ok(_) => Ok(()),
            // The token lapsed/was revoked: re-login once and retry. A
            // second failure is surfaced as-is rather than looping.
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::set(&*client, &self.mount, path, &data)
                    .await
                    .map(|_| ())
                    .map_err(|e| SecretStoreError::Write(e.to_string()))
            }
            Err(e) => Err(SecretStoreError::Write(e.to_string())),
        }
    }

    /// Whether the document already stored at `path` equals `desired`.
    ///
    /// Any failure to read answers `false` so the caller falls through to the
    /// write it was going to do anyway: a missing path (404, the first write),
    /// a lapsed token, an unparseable document. This must never be the reason
    /// a credential fails to land — and a read that failed because the token
    /// expired is handled by `write_path`'s own re-login retry, which runs
    /// immediately after.
    async fn path_matches(&self, path: &str, desired: &serde_json::Value) -> bool {
        // Guard scoped like every other KV call here, so a later `relogin`
        // write lock cannot deadlock against a still-held read guard.
        let current = {
            let client = self.client.read().await;
            kv2::read::<serde_json::Value>(&*client, &self.mount, path).await
        };
        matches!(current, Ok(ref stored) if stored == desired)
    }

    /// Deletes all versions + metadata (not a soft `delete_latest`) at an
    /// **explicit** KV path — matches `K8sSecretStore::delete`'s intent of
    /// actually removing the secret, not leaving a recoverable tombstone.
    /// Idempotent: Vault returns 404 for a path with no metadata, treated
    /// as success the same way `K8sSecretStore::delete` treats a 404 from
    /// the Kubernetes API. An auth failure re-logs-in once and retries,
    /// same as `write_path`.
    pub async fn delete_path(&self, path: &str) -> Result<(), SecretStoreError> {
        // Guard scoped to its own block so `relogin`'s write lock can't
        // deadlock against a still-held read guard (see `write_path`).
        let first = {
            let client = self.client.read().await;
            kv2::delete_metadata(&*client, &self.mount, path).await
        };
        match first {
            Ok(()) => Ok(()),
            Err(ClientError::APIError { code: 404, .. }) => Ok(()),
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                match kv2::delete_metadata(&*client, &self.mount, path).await {
                    Ok(()) => Ok(()),
                    Err(ClientError::APIError { code: 404, .. }) => Ok(()),
                    Err(e) => Err(SecretStoreError::Delete(e.to_string())),
                }
            }
            Err(e) => Err(SecretStoreError::Delete(e.to_string())),
        }
    }
}

/// The default single-destination behavior (no `secretTargets`): derive the
/// path from the CR's namespace/name and funnel through the explicit-path
/// methods above.
#[async_trait::async_trait]
impl SecretStore for VaultSecretStore {
    async fn write_oauth2_credentials(
        &self,
        namespace: &str,
        name: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        self.write_path(&self.default_path(namespace, name), credentials)
            .await
    }

    async fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        self.delete_path(&self.default_path(namespace, name)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_ca_file_materializes_the_pem_then_removes_it_on_drop() {
        let pem = b"-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n";
        let path = {
            let guard = TempCaFile::write(pem).expect("temp CA file write must succeed");
            let path = std::path::PathBuf::from(guard.path_string());
            // Exists and holds exactly the PEM bytes while the guard lives.
            assert!(path.exists(), "temp CA file should exist while guarded");
            assert_eq!(std::fs::read(&path).unwrap(), pem);
            path
        };
        // Dropped at end of scope → file removed.
        assert!(
            !path.exists(),
            "temp CA file should be removed once the guard is dropped"
        );
    }
}
