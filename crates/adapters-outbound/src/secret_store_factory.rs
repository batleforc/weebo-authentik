use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::AuthentikInstance;
use api::instance::{SecretStoreBackend, VaultSecretStoreSpec};
use application::ports::{SecretStore, SecretStoreFactory, SecretStoreFactoryError};
use kube::Client;
use kube::runtime::reflector::Store;
use tokio::sync::Mutex;

use crate::instance_resolver::InstanceResolver;
use crate::secret_k8s::K8sSecretStore;
use crate::secret_vault::VaultSecretStore;

/// Every pod gets a projected ServiceAccount JWT here by default (no extra
/// RBAC or volume config needed beyond what any operator pod already
/// has) — this is what's exchanged for a Vault token via Vault's
/// Kubernetes auth backend.
const SERVICE_ACCOUNT_TOKEN_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// A cached Vault-backed store, valid until its Vault login lease is due
/// for refresh (see `VaultSecretStore::cache_ttl`). `spec` is kept so a
/// `secretStore.vault` edit on the `AuthentikInstance` invalidates the
/// entry immediately rather than waiting out the lease window.
struct CachedVaultStore {
    store: Arc<VaultSecretStore>,
    expires_at: Instant,
    spec: VaultSecretStoreSpec,
}

/// Resolves a `SecretStore` for the target `AuthentikInstance` CR's
/// `spec.secretStore`.
///
/// The `AuthentikInstance` CR is read through an `InstanceResolver` (a live
/// reflector `Store` when wired via `with_instance_store`, a live apiserver
/// call otherwise). The **Vault login is cached**, keyed by `instanceRef`:
/// re-authenticating on every reconcile mints a throwaway Vault token each
/// time, so the built `VaultSecretStore` is reused for the lifetime of its
/// login lease. The Kubernetes backend is a zero-cost wrapper around the
/// shared client and is never cached.
pub struct AuthentikSecretStoreFactory {
    client: Client,
    instances: InstanceResolver,
    vault_cache: Mutex<HashMap<String, CachedVaultStore>>,
}

impl AuthentikSecretStoreFactory {
    pub fn new(client: Client) -> Self {
        Self {
            instances: InstanceResolver::new(client.clone()),
            client,
            vault_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve `AuthentikInstance` CRs from a shared reflector `Store`
    /// instead of a per-call apiserver GET — wired by `operator::main`.
    pub fn with_instance_store(client: Client, store: Store<AuthentikInstance>) -> Self {
        Self {
            instances: InstanceResolver::with_store(client.clone(), store),
            client,
            vault_cache: Mutex::new(HashMap::new()),
        }
    }

    async fn resolve_instance(
        &self,
        instance_ref: &str,
    ) -> Result<AuthentikInstance, SecretStoreFactoryError> {
        self.instances
            .get(instance_ref)
            .await
            .map_err(SecretStoreFactoryError::ResolutionFailed)?
            .ok_or_else(|| SecretStoreFactoryError::InstanceNotFound(instance_ref.to_string()))
    }

    /// Returns the cached Vault store for `instance_ref` if it is still
    /// within its lease window and its spec is unchanged, otherwise logs in
    /// afresh and caches the result. The cache lock is held across the
    /// login so concurrent reconciles of applications sharing an instance
    /// don't each trigger a redundant login.
    async fn vault_store_for(
        &self,
        instance_ref: &str,
        vault_spec: &VaultSecretStoreSpec,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        let mut cache = self.vault_cache.lock().await;

        if let Some(entry) = cache.get(instance_ref)
            && Instant::now() < entry.expires_at
            && &entry.spec == vault_spec
        {
            return Ok(entry.store.clone());
        }

        let jwt = std::fs::read_to_string(SERVICE_ACCOUNT_TOKEN_PATH).map_err(|e| {
            SecretStoreFactoryError::ResolutionFailed(format!(
                "reading this pod's ServiceAccount token at {SERVICE_ACCOUNT_TOKEN_PATH}: {e}"
            ))
        })?;
        let store = Arc::new(
            VaultSecretStore::new(vault_spec, jwt.trim())
                .await
                .map_err(|e| SecretStoreFactoryError::ResolutionFailed(e.to_string()))?,
        );

        // A `None` TTL means the login lease isn't safely cacheable — store
        // it as already-expired so this call still returns it, but the next
        // one re-logs-in.
        let expires_at = Instant::now() + store.cache_ttl().unwrap_or(Duration::ZERO);
        cache.insert(
            instance_ref.to_string(),
            CachedVaultStore {
                store: store.clone(),
                expires_at,
                spec: vault_spec.clone(),
            },
        );
        Ok(store)
    }
}

#[async_trait::async_trait]
impl SecretStoreFactory for AuthentikSecretStoreFactory {
    async fn secret_store_for(
        &self,
        instance_ref: &str,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        let instance = self.resolve_instance(instance_ref).await?;
        match instance.spec.secret_store.backend {
            SecretStoreBackend::Kubernetes => {
                Ok(Arc::new(K8sSecretStore::new(self.client.clone())))
            }
            SecretStoreBackend::Vault => {
                let vault_spec = instance.spec.secret_store.vault.as_ref().ok_or_else(|| {
                    SecretStoreFactoryError::ResolutionFailed(format!(
                        "AuthentikInstance {:?} has secretStore.backend: vault but no \
                         secretStore.vault config",
                        instance.metadata.name
                    ))
                })?;
                self.vault_store_for(instance_ref, vault_spec).await
            }
        }
    }
}
