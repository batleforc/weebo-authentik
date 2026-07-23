use api::AuthentikBrand;
use application::ports::GatewayError;
use authentik_client::apis::core_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::map_err;

impl AuthentikHttpGateway {
    /// Same as `resolve_flow`, for the optional flow slugs on
    /// `AuthentikBrandSpec` (unlike `Oauth2ProviderSpec`'s flows, which
    /// are required).
    async fn resolve_optional_flow(
        &self,
        slug: Option<&str>,
    ) -> Result<Option<uuid::Uuid>, GatewayError> {
        match slug {
            Some(slug) => Ok(Some(self.resolve_flow(slug).await?)),
            None => Ok(None),
        }
    }

    pub(super) async fn create_brand_impl(
        &self,
        brand: &AuthentikBrand,
    ) -> Result<String, GatewayError> {
        let req = models::BrandRequest {
            domain: brand.spec.domain.clone(),
            default: None,
            branding_title: brand.spec.branding_title.clone(),
            branding_logo: brand.spec.branding_logo.clone(),
            branding_favicon: brand.spec.branding_favicon.clone(),
            branding_custom_css: None,
            branding_default_flow_background: brand.spec.branding_default_flow_background.clone(),
            flow_authentication: Some(
                self.resolve_optional_flow(brand.spec.flow_authentication.as_deref())
                    .await?,
            ),
            flow_invalidation: Some(
                self.resolve_optional_flow(brand.spec.flow_invalidation.as_deref())
                    .await?,
            ),
            flow_recovery: Some(
                self.resolve_optional_flow(brand.spec.flow_recovery.as_deref())
                    .await?,
            ),
            flow_unenrollment: Some(
                self.resolve_optional_flow(brand.spec.flow_unenrollment.as_deref())
                    .await?,
            ),
            flow_user_settings: Some(
                self.resolve_optional_flow(brand.spec.flow_user_settings.as_deref())
                    .await?,
            ),
            flow_device_code: None,
            flow_lockdown: None,
            // Not resolved — see the doc comment on this trait method.
            default_application: None,
            web_certificate: None,
            client_certificates: None,
            attributes: None,
        };
        core_api::core_brands_create(&self.configuration, req)
            .await
            .map(|b| b.brand_uuid.to_string())
            .map_err(map_err)
    }

    pub(super) async fn update_brand_impl(
        &self,
        authentik_id: &str,
        brand: &AuthentikBrand,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedBrandRequest {
            domain: Some(brand.spec.domain.clone()),
            default: None,
            branding_title: brand.spec.branding_title.clone(),
            branding_logo: brand.spec.branding_logo.clone(),
            branding_favicon: brand.spec.branding_favicon.clone(),
            branding_custom_css: None,
            branding_default_flow_background: brand.spec.branding_default_flow_background.clone(),
            flow_authentication: Some(
                self.resolve_optional_flow(brand.spec.flow_authentication.as_deref())
                    .await?,
            ),
            flow_invalidation: Some(
                self.resolve_optional_flow(brand.spec.flow_invalidation.as_deref())
                    .await?,
            ),
            flow_recovery: Some(
                self.resolve_optional_flow(brand.spec.flow_recovery.as_deref())
                    .await?,
            ),
            flow_unenrollment: Some(
                self.resolve_optional_flow(brand.spec.flow_unenrollment.as_deref())
                    .await?,
            ),
            flow_user_settings: Some(
                self.resolve_optional_flow(brand.spec.flow_user_settings.as_deref())
                    .await?,
            ),
            flow_device_code: None,
            flow_lockdown: None,
            // Not resolved — see the doc comment on this trait method.
            default_application: None,
            web_certificate: None,
            client_certificates: None,
            attributes: None,
        };
        core_api::core_brands_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    pub(super) async fn delete_brand_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        super::shared::ignore_not_found(
            core_api::core_brands_destroy(&self.configuration, authentik_id).await,
        )
    }

    pub(super) async fn get_default_brand_impl(&self) -> Result<Option<String>, GatewayError> {
        let list = core_api::core_brands_list(
            &self.configuration,
            None,       // brand_uuid
            None,       // branding_default_flow_background
            None,       // branding_favicon
            None,       // branding_logo
            None,       // branding_title
            None,       // client_certificates
            Some(true), // default
            None,       // domain
            None,       // flow_authentication
            None,       // flow_device_code
            None,       // flow_invalidation
            None,       // flow_lockdown
            None,       // flow_recovery
            None,       // flow_unenrollment
            None,       // flow_user_settings
            None,       // ordering
            None,       // page
            None,       // page_size
            None,       // search
            None,       // web_certificate
        )
        .await
        .map_err(map_err)?;
        Ok(list
            .results
            .into_iter()
            .next()
            .map(|b| b.brand_uuid.to_string()))
    }

    pub(super) async fn set_brand_default_impl(
        &self,
        authentik_id: &str,
        default: bool,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedBrandRequest {
            domain: None,
            default: Some(default),
            branding_title: None,
            branding_logo: None,
            branding_favicon: None,
            branding_custom_css: None,
            branding_default_flow_background: None,
            flow_authentication: None,
            flow_invalidation: None,
            flow_recovery: None,
            flow_unenrollment: None,
            flow_user_settings: None,
            flow_device_code: None,
            flow_lockdown: None,
            default_application: None,
            web_certificate: None,
            client_certificates: None,
            attributes: None,
        };
        core_api::core_brands_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }
}
