//! Test-only `SecretStoreFactory`: always returns the one scripted
//! secret store it was built with, ignoring the instance-ref argument
//! entirely — same "bypass real `AuthentikInstance` resolution" intent
//! as `StaticGatewayFactory`.

use std::sync::Arc;

use application::ports::{SecretStore, SecretStoreFactory, SecretStoreFactoryError};

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
    async fn secret_store_for(
        &self,
        _instance_ref: &str,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        Ok(self.store.clone())
    }
}
