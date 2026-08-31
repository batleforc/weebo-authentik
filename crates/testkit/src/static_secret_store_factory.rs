//! Test-only `SecretStoreFactory`: always returns the one scripted
//! secret store it was built with, ignoring the application argument
//! entirely — same "bypass real `AuthentikInstance` resolution" intent
//! as `StaticGatewayFactory`.

use std::sync::Arc;

use api::AuthentikApplication;
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
        _app: &AuthentikApplication,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreFactoryError> {
        Ok(self.store.clone())
    }
}
