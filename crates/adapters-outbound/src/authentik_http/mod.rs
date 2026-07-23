//! Wraps the generated `authentik-client`, split into one file per CRD
//! resource domain (mirroring `api/`, `controller/`, `use_cases/`'s
//! per-resource convention) — this `mod.rs` only holds the type, its
//! constructor, and the single `impl AuthentikGateway` block Rust requires
//! to live in one place, as thin delegators to each resource file's own
//! inherent methods.

mod applications;
mod brands;
mod groups;
mod outposts;
mod policy_bindings;
mod providers;
mod shared;
mod users;

use api::application::{Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec};
use api::{AuthentikApplication, AuthentikBrand, AuthentikGroup, AuthentikOutpost, AuthentikUser};
use application::ports::{
    AuthentikGateway, GatewayError, Oauth2ProviderUpsertResult, RemoteApplication,
};
use authentik_client::apis::configuration::Configuration;

/// One instance per `AuthentikInstance` CR — construction (base path/token
/// pulled from the CR's `tokenSecretRef`) happens in `AuthentikGatewayFactory`
/// (`gateway_factory.rs`), not here.
pub struct AuthentikHttpGateway {
    configuration: Configuration,
}

impl AuthentikHttpGateway {
    pub fn new(base_path: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        let configuration = Configuration {
            base_path: base_path.into(),
            bearer_access_token: Some(bearer_token.into()),
            ..Configuration::default()
        };
        Self { configuration }
    }
}

#[async_trait::async_trait]
impl AuthentikGateway for AuthentikHttpGateway {
    async fn create_application(
        &self,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<String, GatewayError> {
        self.create_application_impl(app, provider_id).await
    }

    async fn update_application(
        &self,
        authentik_id: &str,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<(), GatewayError> {
        self.update_application_impl(authentik_id, app, provider_id)
            .await
    }

    async fn delete_application(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_application_impl(authentik_id).await
    }

    async fn get_application(&self, authentik_id: &str) -> Result<RemoteApplication, GatewayError> {
        self.get_application_impl(authentik_id).await
    }

    async fn delete_provider(
        &self,
        authentik_id: i32,
        kind: ProviderKind,
    ) -> Result<(), GatewayError> {
        self.delete_provider_impl(authentik_id, kind).await
    }

    async fn create_group(&self, group: &AuthentikGroup) -> Result<String, GatewayError> {
        self.create_group_impl(group).await
    }

    async fn update_group(
        &self,
        authentik_id: &str,
        group: &AuthentikGroup,
    ) -> Result<(), GatewayError> {
        self.update_group_impl(authentik_id, group).await
    }

    async fn delete_group(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_group_impl(authentik_id).await
    }

    async fn create_user(&self, user: &AuthentikUser) -> Result<String, GatewayError> {
        self.create_user_impl(user).await
    }

    async fn update_user(
        &self,
        authentik_id: &str,
        user: &AuthentikUser,
    ) -> Result<(), GatewayError> {
        self.update_user_impl(authentik_id, user).await
    }

    async fn delete_user(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_user_impl(authentik_id).await
    }

    async fn create_outpost(&self, outpost: &AuthentikOutpost) -> Result<String, GatewayError> {
        self.create_outpost_impl(outpost).await
    }

    async fn update_outpost(
        &self,
        authentik_id: &str,
        outpost: &AuthentikOutpost,
    ) -> Result<(), GatewayError> {
        self.update_outpost_impl(authentik_id, outpost).await
    }

    async fn delete_outpost(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_outpost_impl(authentik_id).await
    }

    async fn attach_outpost(
        &self,
        outpost_ref: Option<&str>,
        provider_authentik_id: &str,
    ) -> Result<(), GatewayError> {
        self.attach_outpost_impl(outpost_ref, provider_authentik_id)
            .await
    }

    async fn create_brand(&self, brand: &AuthentikBrand) -> Result<String, GatewayError> {
        self.create_brand_impl(brand).await
    }

    async fn update_brand(
        &self,
        authentik_id: &str,
        brand: &AuthentikBrand,
    ) -> Result<(), GatewayError> {
        self.update_brand_impl(authentik_id, brand).await
    }

    async fn delete_brand(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_brand_impl(authentik_id).await
    }

    async fn get_default_brand(&self) -> Result<Option<String>, GatewayError> {
        self.get_default_brand_impl().await
    }

    async fn set_brand_default(
        &self,
        authentik_id: &str,
        default: bool,
    ) -> Result<(), GatewayError> {
        self.set_brand_default_impl(authentik_id, default).await
    }

    async fn upsert_oauth2_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &Oauth2ProviderSpec,
    ) -> Result<Oauth2ProviderUpsertResult, GatewayError> {
        self.upsert_oauth2_provider_impl(authentik_id, name, spec)
            .await
    }

    async fn upsert_proxy_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &ProxyProviderSpec,
    ) -> Result<String, GatewayError> {
        self.upsert_proxy_provider_impl(authentik_id, name, spec)
            .await
    }

    async fn upsert_policy_binding(
        &self,
        authentik_id: Option<&str>,
        application_authentik_id: &str,
        group_name: &str,
        order: i32,
        negate: bool,
    ) -> Result<String, GatewayError> {
        self.upsert_policy_binding_impl(
            authentik_id,
            application_authentik_id,
            group_name,
            order,
            negate,
        )
        .await
    }

    async fn delete_policy_binding(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.delete_policy_binding_impl(authentik_id).await
    }
}
