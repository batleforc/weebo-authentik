use std::sync::Arc;

use api::AuthentikInstance;
use application::ports::{AuthentikGateway, GatewayFactory, GatewayFactoryError};
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::Api;

use crate::authentik_http::AuthentikHttpGateway;

/// Resolves an `AuthentikGateway` fresh on every call by reading the
/// target `AuthentikInstance` CR and its `tokenSecretRef` `Secret` — no
/// caching, see `application::ports::GatewayFactory`.
pub struct AuthentikGatewayFactory {
    client: Client,
}

impl AuthentikGatewayFactory {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn build_gateway(
        &self,
        instance: &AuthentikInstance,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let secret_ref = &instance.spec.token_secret_ref;
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &secret_ref.namespace);
        let secret = secrets.get(&secret_ref.name).await.map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!(
                "fetching secret {}/{}: {e}",
                secret_ref.namespace, secret_ref.name
            ))
        })?;

        let token_bytes = secret
            .data
            .as_ref()
            .and_then(|data| data.get(&secret_ref.key))
            .ok_or_else(|| {
                GatewayFactoryError::ResolutionFailed(format!(
                    "secret {}/{} has no key {:?}",
                    secret_ref.namespace, secret_ref.name, secret_ref.key
                ))
            })?;
        let token = String::from_utf8(token_bytes.0.clone()).map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!(
                "secret {}/{} key {:?} is not valid utf-8: {e}",
                secret_ref.namespace, secret_ref.name, secret_ref.key
            ))
        })?;

        Ok(Arc::new(AuthentikHttpGateway::new(
            instance.spec.url.clone(),
            token,
        )))
    }
}

#[async_trait::async_trait]
impl GatewayFactory for AuthentikGatewayFactory {
    async fn gateway_for(
        &self,
        instance_ref: &str,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let instances: Api<AuthentikInstance> = Api::all(self.client.clone());
        let instance = instances.get_opt(instance_ref).await.map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!(
                "fetching AuthentikInstance {instance_ref:?}: {e}"
            ))
        })?;
        let Some(instance) = instance else {
            return Err(GatewayFactoryError::InstanceNotFound(
                instance_ref.to_string(),
            ));
        };
        self.build_gateway(&instance).await
    }

    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let instances: Api<AuthentikInstance> = Api::all(self.client.clone());
        let list = instances.list(&Default::default()).await.map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!("listing AuthentikInstance: {e}"))
        })?;

        match list.items.len() {
            1 => self.build_gateway(&list.items[0]).await,
            0 => Err(GatewayFactoryError::AmbiguousDefault(
                "no AuthentikInstance CR exists".to_string(),
            )),
            n => Err(GatewayFactoryError::AmbiguousDefault(format!(
                "{n} AuthentikInstance CRs exist, expected exactly one for a default"
            ))),
        }
    }
}
