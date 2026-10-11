//! Builds the per-instance `RoutingSecretStore` (the kit's) from an
//! `AuthentikInstance`'s `secretStore`: every secret target goes to the
//! Kubernetes API or to the instance's Vault. Vault logins are cached per
//! instance by the kit's `VaultLogins`, and only attempted when a target
//! actually routes to Vault — a broken Vault never blocks Kubernetes
//! targets.

use std::sync::Arc;

use api::AuthentikInstance;
use api::instance::VaultSecretStoreSpec;
use application::ports::{SecretStore, SecretStoreError, SecretStoreFactory};
use futures::FutureExt;
use kube::runtime::reflector::Store;
use kube::{Client, ResourceExt};
use weebo_kit_adapters::{
    K8sSecretStore, RoutingSecretStore, VaultConfig, VaultLogins, VaultSecretStore,
};

use crate::instance_resolver::{self, InstanceResolver};
use crate::secret_ref::read_secret_key;

/// Field manager of the `Secret`s this operator emits — the one earlier
/// releases server-side-applied them with, so those Secrets (unlabelled)
/// are adopted rather than reported as foreign.
pub const FIELD_MANAGER: &str = "weebo-authentik-operator";
/// `app.kubernetes.io/managed-by` of the `Secret`s this operator emits.
pub const MANAGED_BY: &str = "weebo-authentik";

/// The Kubernetes secret store, labelled and annotated as this operator's.
pub fn kubernetes_secret_store(client: Client) -> K8sSecretStore {
    K8sSecretStore::new(
        client,
        FIELD_MANAGER,
        MANAGED_BY,
        api::SECRET_OWNER_ANNOTATION,
    )
}

pub struct AuthentikSecretStoreFactory {
    client: Client,
    instances: InstanceResolver,
    kubernetes: Arc<K8sSecretStore>,
    vault: Arc<VaultLogins>,
}

impl AuthentikSecretStoreFactory {
    pub fn new(client: Client) -> Self {
        Self::build(client.clone(), instance_resolver::live(client))
    }

    /// Resolve `AuthentikInstance` CRs from a shared reflector `Store`
    /// instead of a per-call apiserver GET — wired by `operator::main`.
    pub fn with_instance_store(client: Client, store: Store<AuthentikInstance>) -> Self {
        Self::build(client.clone(), instance_resolver::with_store(client, store))
    }

    fn build(client: Client, instances: InstanceResolver) -> Self {
        Self {
            instances,
            kubernetes: Arc::new(kubernetes_secret_store(client.clone())),
            client,
            vault: Arc::new(VaultLogins::default()),
        }
    }
}

/// Logs in to the Vault of `instance` (cached by `logins`). The CA bundle
/// is read first: its digest is part of the cache key, so a rotated CA is
/// picked up at once rather than when the login expires. A configured but
/// unreadable CA is an error, never a fall-back to the system trust store.
async fn vault_for(
    client: Client,
    logins: Arc<VaultLogins>,
    instance: String,
    spec: VaultSecretStoreSpec,
) -> Result<Arc<VaultSecretStore>, String> {
    let ca = match &spec.ca_secret_ref {
        Some(r) => Some(read_secret_key(&client, r, "Vault CA secret").await?),
        None => None,
    };
    let config = VaultConfig {
        address: spec.address.clone(),
        mount: spec.mount.clone(),
        path_prefix: spec.path_prefix.clone(),
        kubernetes_auth_role: spec.kubernetes_auth_role.clone(),
        kubernetes_auth_mount: spec.kubernetes_auth_mount.clone(),
        ca_source: spec
            .ca_secret_ref
            .as_ref()
            .zip(ca.as_ref())
            .map(|(r, pem)| VaultConfig::ca_source_of(r, pem)),
    };
    logins
        .get(&instance, &config, || async move { Ok(ca) })
        .await
}

#[async_trait::async_trait]
impl SecretStoreFactory for AuthentikSecretStoreFactory {
    async fn store_for(
        &self,
        instance_ref: Option<&str>,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreError> {
        // `AuthentikApplication`, the only kind emitting secrets, always
        // names its instance.
        let name = instance_ref
            .ok_or_else(|| SecretStoreError("an instanceRef is required".to_string()))?;
        let instance = self
            .instances
            .get(name)
            .await
            .map_err(SecretStoreError)?
            .ok_or_else(|| SecretStoreError(format!("AuthentikInstance {name:?} not found")))?;
        let store_spec = &instance.spec.secret_store;
        let Some(vault) = store_spec.vault.clone() else {
            return Ok(Arc::new(RoutingSecretStore::new(
                self.kubernetes.clone(),
                Err(format!(
                    "AuthentikInstance {:?} has no secretStore.vault configured",
                    instance.name_any()
                )),
                store_spec.backend,
            )));
        };
        let (client, logins, name) = (self.client.clone(), self.vault.clone(), instance.name_any());
        Ok(Arc::new(RoutingSecretStore::lazy(
            self.kubernetes.clone(),
            Box::new(move || vault_for(client, logins, name, vault).boxed()),
            store_spec.backend,
        )))
    }
}
