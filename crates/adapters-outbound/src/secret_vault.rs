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
    pub async fn new(spec: &VaultSecretStoreSpec, jwt: &str) -> Result<Self, SecretStoreError> {
        let settings = VaultClientSettingsBuilder::default()
            .address(&spec.address)
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

    fn path(&self, namespace: &str, name: &str) -> String {
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

/// A Vault response that means "your token is no good" — 403 (permission
/// denied, Vault's response for an expired/revoked token) or 401. Anything
/// else is a genuine error the caller must see, not retry.
fn is_auth_error(err: &ClientError) -> bool {
    matches!(err, ClientError::APIError { code, .. } if *code == 401 || *code == 403)
}

#[async_trait::async_trait]
impl SecretStore for VaultSecretStore {
    async fn write_oauth2_credentials(
        &self,
        namespace: &str,
        name: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        let data = serde_json::json!({
            "AUTHENTIK_CLIENT_ID": credentials.client_id,
            "AUTHENTIK_CLIENT_SECRET": credentials.client_secret,
            "AUTHENTIK_URL": credentials.authentik_url,
        });
        let path = self.path(namespace, name);

        // The read guard must be released before `relogin` (which takes the
        // write lock), so the KV call is scoped to its own block rather
        // than left as a `match` scrutinee temporary — those live until the
        // end of the match and would deadlock the re-login.
        let first = {
            let client = self.client.read().await;
            kv2::set(&*client, &self.mount, &path, &data).await
        };
        match first {
            Ok(_) => Ok(()),
            // The token lapsed/was revoked: re-login once and retry. A
            // second failure is surfaced as-is rather than looping.
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::set(&*client, &self.mount, &path, &data)
                    .await
                    .map(|_| ())
                    .map_err(|e| SecretStoreError::Write(e.to_string()))
            }
            Err(e) => Err(SecretStoreError::Write(e.to_string())),
        }
    }

    /// Deletes all versions + metadata (not a soft `delete_latest`) —
    /// matches `K8sSecretStore::delete`'s intent of actually removing the
    /// secret, not leaving a recoverable tombstone. Idempotent: Vault
    /// returns 404 for a path with no metadata, treated as success the
    /// same way `K8sSecretStore::delete` treats a 404 from the
    /// Kubernetes API. An auth failure re-logs-in once and retries, same
    /// as `write_oauth2_credentials`.
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        let path = self.path(namespace, name);
        // Guard scoped to its own block so `relogin`'s write lock can't
        // deadlock against a still-held read guard (see
        // `write_oauth2_credentials`).
        let first = {
            let client = self.client.read().await;
            kv2::delete_metadata(&*client, &self.mount, &path).await
        };
        match first {
            Ok(()) => Ok(()),
            Err(ClientError::APIError { code: 404, .. }) => Ok(()),
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                match kv2::delete_metadata(&*client, &self.mount, &path).await {
                    Ok(()) => Ok(()),
                    Err(ClientError::APIError { code: 404, .. }) => Ok(()),
                    Err(e) => Err(SecretStoreError::Delete(e.to_string())),
                }
            }
            Err(e) => Err(SecretStoreError::Delete(e.to_string())),
        }
    }
}
