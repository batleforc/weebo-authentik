//! Test-only `GatewayFactory`: always returns the one scripted gateway
//! it was built with, ignoring the instance-ref argument entirely. This
//! is what "swap `AuthentikGateway` for the wiremock mock" means in
//! practice for a controller integration test — no real
//! `AuthentikInstance` CR/Secret is ever needed, `AuthentikGatewayFactory`
//! resolution is bypassed outright.

use std::sync::Arc;

use api::application::{InstanceKind, InstanceRef};
use application::ports::{
    AuthentikGateway, BoundInstance, GatewayFactory, GatewayFactoryError, InstanceDirectory,
    InstanceError, ResolveError,
};

pub struct StaticGatewayFactory {
    gateway: Arc<dyn AuthentikGateway>,
}

impl StaticGatewayFactory {
    pub fn new(gateway: Arc<dyn AuthentikGateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait::async_trait]
impl GatewayFactory for StaticGatewayFactory {
    async fn gateway_for(
        &self,
        _instance: &BoundInstance,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        Ok(self.gateway.clone())
    }

    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        Ok(self.gateway.clone())
    }
}

/// Every `instanceRef` resolves: to an `AuthentikLocalInstance` when it
/// says so, to an `AuthentikInstance` of that name otherwise — no instance
/// CR is needed.
#[async_trait::async_trait]
impl InstanceDirectory for StaticGatewayFactory {
    async fn resolve(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        _recorded: Option<&BoundInstance>,
    ) -> Result<BoundInstance, InstanceError> {
        let Some(r) = instance_ref else {
            return Err(ResolveError::NoInstance.into());
        };
        Ok(match (r.kind, r.namespace.as_deref().or(cr_namespace)) {
            (Some(InstanceKind::LocalInstance), Some(ns)) => BoundInstance::local(ns, &r.name),
            _ => BoundInstance::cluster(&r.name),
        })
    }
}
