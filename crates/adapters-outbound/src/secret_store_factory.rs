use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::application::SecretTarget;
use api::instance::{SecretStoreBackend, VaultSecretStoreSpec};
use api::{AuthentikApplication, AuthentikInstance};
use application::ports::{SecretStore, SecretStoreFactory, SecretStoreFactoryError};
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::Api;
use kube::runtime::reflector::Store;
use tokio::sync::Mutex;

use crate::instance_resolver::InstanceResolver;
use crate::secret_fanout::FanOutSecretStore;
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

/// Resolves a `SecretStore` for the target `AuthentikApplication` CR.
///
/// Routing is driven by the application's `spec.secretTargets`:
/// - **empty** (the default) → the single destination defined by the
///   owning `AuthentikInstance`'s `spec.secretStore` (a Kubernetes
///   `Secret`, or the instance's Vault path convention) — the historical
///   behavior, unchanged.
/// - **non-empty** → a `FanOutSecretStore` writing to every listed Vault
///   path / Kubernetes Secret, mixing backends freely.
///
/// The `AuthentikInstance` CR is read through an `InstanceResolver` (a live
/// reflector `Store` when wired via `with_instance_store`, a live apiserver
/// call otherwise) and always supplies the Vault **connection** (address,
/// mount, Kubernetes-auth role) that both the default Vault destination and
/// any Vault target reuse. The **Vault login is cached**, keyed by
/// `instanceRef`: re-authenticating on every reconcile mints a throwaway
/// Vault token each time, so the built `VaultSecretStore` connection is
/// reused for the lifetime of its login lease. The per-application fan-out
/// wrapper and the Kubernetes backend are zero-cost and never cached.
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

    /// The Vault `secretStore.vault` connection config of `instance` — a
    /// Vault destination (the instance default or an application target)
    /// needs it, so its absence is a misconfiguration surfaced as
    /// `ResolutionFailed` rather than a silent skip.
    fn require_vault_spec<'a>(
        &self,
        instance: &'a AuthentikInstance,
    ) -> Result<&'a VaultSecretStoreSpec, SecretStoreFactoryError> {
        instance.spec.secret_store.vault.as_ref().ok_or_else(|| {
            SecretStoreFactoryError::ResolutionFailed(format!(
                "AuthentikInstance {:?} is asked for a Vault secret destination but has no \
                     secretStore.vault config",
                instance.metadata.name
            ))
        })
    }

    /// Reads the optional `spec.ca_secret_ref` CA PEM from its Kubernetes
    /// `Secret`, returning `None` when no CA is configured. A configured-but-
    /// unreadable CA (missing Secret/key) is a hard error rather than a
    /// silent fall-back to the system trust store — trusting the wrong roots
    /// against a private-CA Vault is a security regression, not a warning.
    async fn read_vault_ca(
        &self,
        vault_spec: &VaultSecretStoreSpec,
    ) -> Result<Option<Vec<u8>>, SecretStoreFactoryError> {
        let Some(ca_ref) = vault_spec.ca_secret_ref.as_ref() else {
            return Ok(None);
        };
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &ca_ref.namespace);
        let secret = secrets.get(&ca_ref.name).await.map_err(|e| {
            SecretStoreFactoryError::ResolutionFailed(format!(
                "fetching Vault CA secret {}/{}: {e}",
                ca_ref.namespace, ca_ref.name
            ))
        })?;
        let pem = secret
            .data
            .as_ref()
            .and_then(|data| data.get(&ca_ref.key))
            .ok_or_else(|| {
                SecretStoreFactoryError::ResolutionFailed(format!(
                    "Vault CA secret {}/{} has no key {:?}",
                    ca_ref.namespace, ca_ref.name, ca_ref.key
                ))
            })?;
        Ok(Some(pem.0.clone()))
    }

    /// Returns the cached authenticated Vault connection for `instance_ref`
    /// if it is still within its lease window and its spec is unchanged,
    /// otherwise logs in afresh and caches the result. The cache lock is
    /// held across the login so concurrent reconciles of applications
    /// sharing an instance don't each trigger a redundant login. The
    /// returned `VaultSecretStore` is a *connection*: several destinations
    /// (default path, or multiple `secretTargets` paths) share one.
    async fn vault_connection_for(
        &self,
        instance_ref: &str,
        vault_spec: &VaultSecretStoreSpec,
    ) -> Result<Arc<VaultSecretStore>, SecretStoreFactoryError> {
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
        // A custom CA (e.g. openbao-tls) is read from its Secret via the
        // API and handed to the Vault client — no mounted volume needed.
        let ca_pem = self.read_vault_ca(vault_spec).await?;
        let store = Arc::new(
            VaultSecretStore::new(vault_spec, jwt.trim(), ca_pem.as_deref())
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

    /// The single instance-default destination used when an application
    /// declares no `secretTargets` — preserves the historical behavior
    /// exactly (Kubernetes Secret named after the CR, or the instance's
    /// derived Vault path).
    async fn default_store_for(
        &self,
        instance_ref: &str,
        instance: &AuthentikInstance,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        match instance.spec.secret_store.backend {
            SecretStoreBackend::Kubernetes => {
                Ok(Arc::new(K8sSecretStore::new(self.client.clone())))
            }
            SecretStoreBackend::Vault => {
                let vault_spec = self.require_vault_spec(instance)?;
                let store = self.vault_connection_for(instance_ref, vault_spec).await?;
                Ok(store)
            }
        }
    }

    /// A `FanOutSecretStore` covering every `secretTargets` entry, each
    /// destination fully resolved (path/name) from the target plus the
    /// application's namespace/name.
    async fn fanout_store_for(
        &self,
        instance_ref: &str,
        instance: &AuthentikInstance,
        namespace: &str,
        name: &str,
        targets: &[SecretTarget],
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        let mut fanout = FanOutSecretStore::new();
        for target in targets {
            match target.backend {
                SecretStoreBackend::Kubernetes => {
                    let secret_name = target.name.clone().unwrap_or_else(|| name.to_string());
                    fanout.push_kubernetes(
                        K8sSecretStore::new(self.client.clone()),
                        namespace.to_string(),
                        secret_name,
                    );
                }
                SecretStoreBackend::Vault => {
                    let vault_spec = self.require_vault_spec(instance)?;
                    let store = self.vault_connection_for(instance_ref, vault_spec).await?;
                    let path = target
                        .path
                        .clone()
                        .unwrap_or_else(|| store.default_path(namespace, name));
                    fanout.push_vault(store, path);
                }
            }
        }
        Ok(Arc::new(fanout))
    }
}

#[async_trait::async_trait]
impl SecretStoreFactory for AuthentikSecretStoreFactory {
    async fn secret_store_for(
        &self,
        app: &AuthentikApplication,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        let instance_ref = &app.spec.instance_ref;
        let instance = self.resolve_instance(instance_ref).await?;

        if app.spec.secret_targets.is_empty() {
            return self.default_store_for(instance_ref, &instance).await;
        }

        // Same namespace/name derivation the reconciler uses when writing
        // (see `reconcile_application`): the CR's namespace, and its name
        // falling back to the slug.
        let namespace = app.metadata.namespace.clone().unwrap_or_default();
        let name = app
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| app.spec.slug.clone());
        self.fanout_store_for(
            instance_ref,
            &instance,
            &namespace,
            &name,
            &app.spec.secret_targets,
        )
        .await
    }
}
