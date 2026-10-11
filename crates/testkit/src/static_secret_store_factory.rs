//! Test-only `SecretStoreFactory`: always returns the one secret store it
//! was built with, ignoring the `instanceRef` entirely — same "bypass real
//! `AuthentikInstance` resolution" intent as `StaticGatewayFactory`.

use std::sync::Arc;

use application::ports::{SecretStore, SecretStoreError, SecretStoreFactory};

pub struct StaticSecretStoreFactory {
    store: Arc<dyn SecretStore>,
}

impl StaticSecretStoreFactory {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl SecretStoreFactory for StaticSecretStoreFactory {
    async fn store_for(
        &self,
        _instance_ref: Option<&str>,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreError> {
        Ok(self.store.clone())
    }
}
