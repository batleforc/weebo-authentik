use api::AuthentikApplication;
use application::ports::{GatewayError, RemoteApplication};
use authentik_client::apis::core_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err};

impl AuthentikHttpGateway {
    pub(super) async fn create_application_impl(
        &self,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<String, GatewayError> {
        let req = models::ApplicationRequest {
            name: app.spec.name.clone(),
            slug: app.spec.slug.clone(),
            provider: Some(Some(provider_id)),
            backchannel_providers: None,
            open_in_new_tab: None,
            meta_launch_url: None,
            meta_icon: app.spec.meta_icon.clone(),
            meta_description: None,
            meta_publisher: None,
            policy_engine_mode: None,
            group: None,
            meta_hide: None,
        };
        core_api::core_applications_create(&self.configuration, req)
            .await
            .map(|a| a.slug)
            .map_err(map_err)
    }

    pub(super) async fn update_application_impl(
        &self,
        authentik_id: &str,
        app: &AuthentikApplication,
        provider_id: i32,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedApplicationRequest {
            name: Some(app.spec.name.clone()),
            slug: Some(app.spec.slug.clone()),
            provider: Some(Some(provider_id)),
            backchannel_providers: None,
            open_in_new_tab: None,
            meta_launch_url: None,
            meta_icon: app.spec.meta_icon.clone(),
            meta_description: None,
            meta_publisher: None,
            policy_engine_mode: None,
            group: None,
            meta_hide: None,
        };
        core_api::core_applications_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    pub(super) async fn delete_application_impl(
        &self,
        authentik_id: &str,
    ) -> Result<(), GatewayError> {
        ignore_not_found(
            core_api::core_applications_destroy(&self.configuration, authentik_id).await,
        )
    }

    pub(super) async fn get_application_impl(
        &self,
        authentik_id: &str,
    ) -> Result<RemoteApplication, GatewayError> {
        let app = core_api::core_applications_retrieve(&self.configuration, authentik_id)
            .await
            .map_err(map_err)?;
        Ok(RemoteApplication {
            provider_id: app.provider.flatten(),
            provider_meta_model_name: app.provider_obj.map(|p| p.meta_model_name),
        })
    }
}
