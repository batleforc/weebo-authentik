use std::sync::Arc;

use api::AuthentikInstance;
use application::ports::{AuthentikGateway, GatewayFactory, GatewayFactoryError};
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::Api;
use kube::runtime::reflector::Store;

use crate::authentik_http::AuthentikHttpGateway;
use crate::instance_resolver::InstanceResolver;

/// Resolves an `AuthentikGateway` for a given `AuthentikInstance` CR.
///
/// The `AuthentikInstance` itself is read through an `InstanceResolver` —
/// served from a live reflector `Store` when one is wired in (see
/// `with_instance_store`), or a live apiserver call otherwise. The
/// `tokenSecretRef` `Secret` is still read fresh on every call so token
/// rotation is picked up immediately, and the built HTTP gateway is not
/// cached.
pub struct AuthentikGatewayFactory {
    client: Client,
    instances: InstanceResolver,
}

impl AuthentikGatewayFactory {
    pub fn new(client: Client) -> Self {
        Self {
            instances: InstanceResolver::new(client.clone()),
            client,
        }
    }

    /// Resolve `AuthentikInstance` CRs from a shared reflector `Store`
    /// instead of a per-call apiserver GET/LIST — wired by
    /// `operator::main`, which owns the reflector that keeps `store` fresh.
    pub fn with_instance_store(client: Client, store: Store<AuthentikInstance>) -> Self {
        Self {
            instances: InstanceResolver::with_store(client.clone(), store),
            client,
        }
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
        let instance = self
            .instances
            .get(instance_ref)
            .await
            .map_err(GatewayFactoryError::ResolutionFailed)?
            .ok_or_else(|| GatewayFactoryError::InstanceNotFound(instance_ref.to_string()))?;
        self.build_gateway(&instance).await
    }

    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let instances = self
            .instances
            .list()
            .await
            .map_err(GatewayFactoryError::ResolutionFailed)?;

        match instances.len() {
            1 => self.build_gateway(&instances[0]).await,
            0 => Err(GatewayFactoryError::AmbiguousDefault(
                "no AuthentikInstance CR exists".to_string(),
            )),
            n => Err(GatewayFactoryError::AmbiguousDefault(format!(
                "{n} AuthentikInstance CRs exist, expected exactly one for a default"
            ))),
        }
    }
}
