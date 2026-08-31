//! Fan-out `SecretStore`: writes (and deletes) one application's oauth2
//! credentials to **several** pre-resolved destinations at once — any mix
//! of Kubernetes `Secret`s and Vault KV paths. Built per
//! `AuthentikApplication` by `AuthentikSecretStoreFactory` when the CR
//! declares explicit `spec.secretTargets`; an empty `secretTargets` never
//! reaches here (the factory returns the single instance-default store
//! instead).
//!
//! Each destination is fully resolved at build time (the factory already
//! has the CR's namespace/name and each target's explicit path/name), so
//! the `namespace`/`name` arguments of the `SecretStore` trait methods are
//! unused here — routing is baked into `destinations`, not re-derived per
//! call.

use std::sync::Arc;

use application::ports::{Oauth2Credentials, SecretStore, SecretStoreError};

use crate::secret_k8s::K8sSecretStore;
use crate::secret_vault::VaultSecretStore;

/// One resolved write target, abstracted so the fan-out doesn't branch on
/// backend and so its aggregation logic is unit-testable with a fake. Each
/// implementor already carries its full destination coordinates.
#[async_trait::async_trait]
trait BoundTarget: Send + Sync {
    async fn write(&self, credentials: &Oauth2Credentials) -> Result<(), SecretStoreError>;
    async fn delete(&self) -> Result<(), SecretStoreError>;
    /// Human-readable target for error messages — which of the several
    /// destinations failed, without leaking credential values.
    fn label(&self) -> String;
}

/// A Kubernetes `Secret` in `namespace` named `name`.
struct KubeTarget {
    store: K8sSecretStore,
    namespace: String,
    name: String,
}

#[async_trait::async_trait]
impl BoundTarget for KubeTarget {
    async fn write(&self, credentials: &Oauth2Credentials) -> Result<(), SecretStoreError> {
        self.store
            .write_oauth2_credentials(&self.namespace, &self.name, credentials)
            .await
    }
    async fn delete(&self) -> Result<(), SecretStoreError> {
        self.store.delete(&self.namespace, &self.name).await
    }
    fn label(&self) -> String {
        format!("kubernetes secret {}/{}", self.namespace, self.name)
    }
}

/// A Vault KV path. The `Arc` is shared with the per-instance
/// authenticated `VaultSecretStore` (so two Vault targets on the same
/// instance reuse one login), pinned to its own `path`.
struct VaultTarget {
    store: Arc<VaultSecretStore>,
    path: String,
}

#[async_trait::async_trait]
impl BoundTarget for VaultTarget {
    async fn write(&self, credentials: &Oauth2Credentials) -> Result<(), SecretStoreError> {
        self.store.write_path(&self.path, credentials).await
    }
    async fn delete(&self) -> Result<(), SecretStoreError> {
        self.store.delete_path(&self.path).await
    }
    fn label(&self) -> String {
        format!("vault path {}", self.path)
    }
}

/// A `SecretStore` that fans a single write/delete out to every configured
/// destination.
pub struct FanOutSecretStore {
    destinations: Vec<Box<dyn BoundTarget>>,
}

impl FanOutSecretStore {
    /// Build an empty fan-out; the factory pushes one destination per
    /// `secretTargets` entry via the `push_*` helpers.
    pub fn new() -> Self {
        Self {
            destinations: Vec::new(),
        }
    }

    pub fn push_kubernetes(&mut self, store: K8sSecretStore, namespace: String, name: String) {
        self.destinations.push(Box::new(KubeTarget {
            store,
            namespace,
            name,
        }));
    }

    pub fn push_vault(&mut self, store: Arc<VaultSecretStore>, path: String) {
        self.destinations
            .push(Box::new(VaultTarget { store, path }));
    }

    pub fn is_empty(&self) -> bool {
        self.destinations.is_empty()
    }
}

impl Default for FanOutSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SecretStore for FanOutSecretStore {
    /// Writes to every destination in order. Stops at the first failure —
    /// a partially-written fan-out surfaces as a reconcile error and is
    /// retried, re-attempting every destination (writes are idempotent
    /// upserts), so a transient failure on target 2 doesn't leave target 1
    /// permanently stale once the reconcile eventually succeeds.
    async fn write_oauth2_credentials(
        &self,
        _namespace: &str,
        _name: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        for dest in &self.destinations {
            dest.write(credentials).await.map_err(|e| {
                SecretStoreError::Write(format!("writing to {}: {e}", dest.label()))
            })?;
        }
        Ok(())
    }

    /// Attempts **every** destination even if one fails (best-effort
    /// cleanup — a Secret already gone shouldn't block deleting the
    /// remaining Vault paths), then returns the first error, if any. Each
    /// destination's own `delete` is idempotent on a 404, so a retry after
    /// a partial failure is safe.
    async fn delete(&self, _namespace: &str, _name: &str) -> Result<(), SecretStoreError> {
        let mut first_err: Option<SecretStoreError> = None;
        for dest in &self.destinations {
            if let Err(e) = dest.delete().await {
                let e = SecretStoreError::Delete(format!("deleting {}: {e}", dest.label()));
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Records the calls it received and optionally fails, so the fan-out's
    /// ordering and error-aggregation can be asserted without a real
    /// Kubernetes/Vault backend.
    struct FakeTarget {
        label: String,
        fail_write: bool,
        fail_delete: bool,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl BoundTarget for FakeTarget {
        async fn write(&self, _credentials: &Oauth2Credentials) -> Result<(), SecretStoreError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("write:{}", self.label));
            if self.fail_write {
                Err(SecretStoreError::Write(format!("boom {}", self.label)))
            } else {
                Ok(())
            }
        }
        async fn delete(&self) -> Result<(), SecretStoreError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("delete:{}", self.label));
            if self.fail_delete {
                Err(SecretStoreError::Delete(format!("boom {}", self.label)))
            } else {
                Ok(())
            }
        }
        fn label(&self) -> String {
            self.label.clone()
        }
    }

    fn creds() -> Oauth2Credentials {
        Oauth2Credentials {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            authentik_url: "https://authentik.example.com".to_string(),
        }
    }

    fn store_with(targets: Vec<FakeTarget>) -> FanOutSecretStore {
        FanOutSecretStore {
            destinations: targets
                .into_iter()
                .map(|t| Box::new(t) as Box<dyn BoundTarget>)
                .collect(),
        }
    }

    #[tokio::test]
    async fn write_hits_every_destination_in_order() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let store = store_with(vec![
            FakeTarget {
                label: "a".into(),
                fail_write: false,
                fail_delete: false,
                calls: calls.clone(),
            },
            FakeTarget {
                label: "b".into(),
                fail_write: false,
                fail_delete: false,
                calls: calls.clone(),
            },
        ]);

        store
            .write_oauth2_credentials("ns", "name", &creds())
            .await
            .expect("all writes succeed");

        assert_eq!(&*calls.lock().unwrap(), &["write:a", "write:b"]);
    }

    #[tokio::test]
    async fn write_stops_at_first_failure_and_labels_it() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let store = store_with(vec![
            FakeTarget {
                label: "a".into(),
                fail_write: true,
                fail_delete: false,
                calls: calls.clone(),
            },
            FakeTarget {
                label: "b".into(),
                fail_write: false,
                fail_delete: false,
                calls: calls.clone(),
            },
        ]);

        let err = store
            .write_oauth2_credentials("ns", "name", &creds())
            .await
            .expect_err("first target fails");

        // Only the first target was attempted, and the error names it.
        assert_eq!(&*calls.lock().unwrap(), &["write:a"]);
        assert!(err.to_string().contains("writing to a"), "{err}");
    }

    #[tokio::test]
    async fn delete_attempts_all_even_after_a_failure() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let store = store_with(vec![
            FakeTarget {
                label: "a".into(),
                fail_write: false,
                fail_delete: true,
                calls: calls.clone(),
            },
            FakeTarget {
                label: "b".into(),
                fail_write: false,
                fail_delete: false,
                calls: calls.clone(),
            },
        ]);

        let err = store
            .delete("ns", "name")
            .await
            .expect_err("first target's delete fails");

        // Both targets were attempted despite the first failing, and the
        // surfaced error is the first one.
        assert_eq!(&*calls.lock().unwrap(), &["delete:a", "delete:b"]);
        assert!(err.to_string().contains("deleting a"), "{err}");
    }
}
