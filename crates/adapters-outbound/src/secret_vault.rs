use application::ports::{Oauth2Credentials, SecretStore, SecretStoreError};
use vaultrs::client::VaultClient;

/// KV mount/path convention deliberately not decided yet (flagged open in
/// `.prompt/plan.md`) — decide when actually implementing this adapter,
/// ideally aligned with the existing Vault ACL structure from
/// `vault.tf`'s OIDC federation rather than inventing a new one.
#[allow(dead_code)]
pub struct VaultSecretStore {
    client: VaultClient,
}

impl VaultSecretStore {
    pub fn new(client: VaultClient) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl SecretStore for VaultSecretStore {
    async fn write_oauth2_credentials(
        &self,
        _namespace: &str,
        _name: &str,
        _credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        todo!("phase 2 — KV mount/path convention not decided yet, see .prompt/plan.md")
    }

    async fn delete(&self, _namespace: &str, _name: &str) -> Result<(), SecretStoreError> {
        todo!("phase 2")
    }
}
