use std::sync::Arc;

use api::AuthentikInstance;
use api::instance::SecretStoreBackend;
use application::ports::{SecretStore, SecretStoreFactory, SecretStoreFactoryError};
use kube::Client;
use kube::api::Api;

use crate::secret_k8s::K8sSecretStore;
use crate::secret_vault::VaultSecretStore;

/// Every pod gets a projected ServiceAccount JWT here by default (no extra
/// RBAC or volume config needed beyond what any operator pod already
/// has) — this is what's exchanged for a Vault token via Vault's
/// Kubernetes auth backend.
const SERVICE_ACCOUNT_TOKEN_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// Resolves a `SecretStore` fresh on every call by reading the target
/// `AuthentikInstance` CR's `spec.secretStore` — no caching, same
/// rationale as `AuthentikGatewayFactory`. See
/// `application::ports::SecretStoreFactory`.
pub struct AuthentikSecretStoreFactory {
    client: Client,
}

impl AuthentikSecretStoreFactory {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn build_secret_store(
        &self,
        instance: &AuthentikInstance,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        match instance.spec.secret_store.backend {
            SecretStoreBackend::Kubernetes => {
                Ok(Arc::new(K8sSecretStore::new(self.client.clone())))
            }
            SecretStoreBackend::Vault => {
                let vault = instance.spec.secret_store.vault.as_ref().ok_or_else(|| {
                    SecretStoreFactoryError::ResolutionFailed(format!(
                        "AuthentikInstance {:?} has secretStore.backend: vault but no \
                         secretStore.vault config",
                        instance.metadata.name
                    ))
                })?;
                let jwt = std::fs::read_to_string(SERVICE_ACCOUNT_TOKEN_PATH).map_err(|e| {
                    SecretStoreFactoryError::ResolutionFailed(format!(
                        "reading this pod's ServiceAccount token at \
                         {SERVICE_ACCOUNT_TOKEN_PATH}: {e}"
                    ))
                })?;
                let store = VaultSecretStore::new(vault, jwt.trim())
                    .await
                    .map_err(|e| SecretStoreFactoryError::ResolutionFailed(e.to_string()))?;
                Ok(Arc::new(store))
            }
        }
    }
}

#[async_trait::async_trait]
impl SecretStoreFactory for AuthentikSecretStoreFactory {
    async fn secret_store_for(
        &self,
        instance_ref: &str,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        let instances: Api<AuthentikInstance> = Api::all(self.client.clone());
        let instance = instances.get_opt(instance_ref).await.map_err(|e| {
            SecretStoreFactoryError::ResolutionFailed(format!(
                "fetching AuthentikInstance {instance_ref:?}: {e}"
            ))
        })?;
        let Some(instance) = instance else {
            return Err(SecretStoreFactoryError::InstanceNotFound(
                instance_ref.to_string(),
            ));
        };
        self.build_secret_store(&instance).await
    }
}
