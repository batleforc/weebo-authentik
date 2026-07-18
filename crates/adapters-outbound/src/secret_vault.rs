use application::ports::{Oauth2Credentials, SecretStore, SecretStoreError};
use vaultrs::client::{Client as _, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;
use vaultrs::{auth::kubernetes, kv2};

/// KV v2 mount/path convention (previously open in `.prompt/plan.md`):
/// one secret per `AuthentikApplication` CR at
/// `<mount>/data/<path_prefix>/<namespace>/<name>`, mirroring
/// `K8sSecretStore`'s one-Secret-per-CR, same-name-same-namespace
/// convention. Built fresh per `AuthentikInstance` by
/// `AuthentikSecretStoreFactory::build_secret_store`, same
/// no-caching/always-fresh-auth rationale as `AuthentikGatewayFactory`.
pub struct VaultSecretStore {
    client: VaultClient,
    mount: String,
    path_prefix: String,
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
    pub async fn login(
        address: &str,
        mount: &str,
        path_prefix: &str,
        kubernetes_auth_role: &str,
        kubernetes_auth_mount: &str,
        jwt: &str,
    ) -> Result<Self, SecretStoreError> {
        let settings = VaultClientSettingsBuilder::default()
            .address(address)
            .build()
            .map_err(|e| SecretStoreError::Write(format!("building Vault client settings: {e}")))?;
        let mut client = VaultClient::new(settings)
            .map_err(|e| SecretStoreError::Write(format!("building Vault client: {e}")))?;

        let auth_info = kubernetes::login(
            &client,
            kubernetes_auth_mount,
            kubernetes_auth_role,
            jwt.trim(),
        )
        .await
        .map_err(|e| {
            SecretStoreError::Write(format!(
                "Vault Kubernetes auth login (mount {kubernetes_auth_mount:?}, role \
                 {kubernetes_auth_role:?}) failed: {e}"
            ))
        })?;
        client.set_token(&auth_info.client_token);

        Ok(Self {
            client,
            mount: mount.to_string(),
            path_prefix: path_prefix.to_string(),
        })
    }

    fn path(&self, namespace: &str, name: &str) -> String {
        format!("{}/{namespace}/{name}", self.path_prefix)
    }
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
        kv2::set(
            &self.client,
            &self.mount,
            &self.path(namespace, name),
            &data,
        )
        .await
        .map_err(|e| SecretStoreError::Write(e.to_string()))?;
        Ok(())
    }

    /// Deletes all versions + metadata (not a soft `delete_latest`) —
    /// matches `K8sSecretStore::delete`'s intent of actually removing the
    /// secret, not leaving a recoverable tombstone. Idempotent: Vault
    /// returns 404 for a path with no metadata, treated as success the
    /// same way `K8sSecretStore::delete` treats a 404 from the
    /// Kubernetes API.
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        match kv2::delete_metadata(&self.client, &self.mount, &self.path(namespace, name)).await {
            Ok(()) => Ok(()),
            Err(ClientError::APIError { code: 404, .. }) => Ok(()),
            Err(e) => Err(SecretStoreError::Delete(e.to_string())),
        }
    }
}
