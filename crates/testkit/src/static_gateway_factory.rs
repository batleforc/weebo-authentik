//! Test-only `GatewayFactory`: always returns the one scripted gateway
//! it was built with, ignoring the instance-ref argument entirely. This
//! is what "swap `AuthentikGateway` for the wiremock mock" means in
//! practice for a controller integration test — no real
//! `AuthentikInstance` CR/Secret is ever needed, `AuthentikGatewayFactory`
//! resolution is bypassed outright.

use std::sync::Arc;

use application::ports::{AuthentikGateway, GatewayFactory, GatewayFactoryError};

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
        _instance_ref: &str,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        Ok(self.gateway.clone())
    }

    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        Ok(self.gateway.clone())
    }
}
