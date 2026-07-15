use api::application::{MatchingMode, Oauth2ProviderSpec, ProxyProviderSpec};
use api::outpost::OutpostType;
use api::{AuthentikApplication, AuthentikBrand, AuthentikGroup, AuthentikOutpost, AuthentikUser};
use application::ports::{
    AuthentikGateway, GatewayError, Oauth2Credentials, Oauth2ProviderUpsertResult,
};
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::{
    core_api, crypto_api, flows_api, outposts_api, policies_api, propertymappings_api,
    providers_api,
};
use authentik_client::models;

/// Wraps the generated `authentik-client`. One instance per
/// `AuthentikInstance` CR — construction (base path/token pulled from the
/// CR's `tokenSecretRef`) happens in `AuthentikGatewayFactory`
/// (`gateway_factory.rs`), not here.
pub struct AuthentikHttpGateway {
    configuration: Configuration,
}

/// Authentik's outpost embedded with every installation, never created by
/// this operator — the default target for a proxy provider whose
/// `outpostRef` is unset. See `.prompt/plan.md`.
const EMBEDDED_OUTPOST_NAME: &str = "authentik Embedded Outpost";

impl AuthentikHttpGateway {
    pub fn new(base_path: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        let configuration = Configuration {
            base_path: base_path.into(),
            bearer_access_token: Some(bearer_token.into()),
            ..Configuration::default()
        };
        Self { configuration }
    }

    /// Flows are referenced by slug (no `AuthentikFlow` CRD in v1, see
    /// `.prompt/plan.md`) — resolved to a pk directly against the
    /// Authentik API on every reconcile.
    async fn resolve_flow(&self, slug: &str) -> Result<uuid::Uuid, GatewayError> {
        let list = flows_api::flows_instances_list(
            &self.configuration,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(slug),
        )
        .await
        .map_err(map_err)?;
        list.results
            .into_iter()
            .find(|f| f.slug == slug)
            .map(|f| f.pk)
            .ok_or_else(|| GatewayError::NotFound(format!("flow {slug:?} not found")))
    }

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

    /// Certificate key pairs are always a lookup of an existing
    /// cert (e.g. "authentik Self-signed Certificate"), never created by
    /// this operator. See `.prompt/plan.md`.
    async fn resolve_certificate(&self, name: &str) -> Result<uuid::Uuid, GatewayError> {
        let list = crypto_api::crypto_certificatekeypairs_list(
            &self.configuration,
            None,
            None,
            None,
            Some(name),
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(map_err)?;
        list.results
            .into_iter()
            .find(|c| c.name == name)
            .map(|c| c.pk)
            .ok_or_else(|| GatewayError::NotFound(format!("certificate {name:?} not found")))
    }

    async fn resolve_property_mappings(
        &self,
        names: &[String],
    ) -> Result<Vec<uuid::Uuid>, GatewayError> {
        let mut pks = Vec::with_capacity(names.len());
        for name in names {
            let list = propertymappings_api::propertymappings_all_list(
                &self.configuration,
                None,
                None,
                Some(name),
                None,
                None,
                None,
                None,
            )
            .await
            .map_err(map_err)?;
            let pk = list
                .results
                .into_iter()
                .find(|m| &m.name == name)
                .map(|m| m.pk)
                .ok_or_else(|| {
                    GatewayError::NotFound(format!("property mapping {name:?} not found"))
                })?;
            pks.push(pk);
        }
        Ok(pks)
    }

    /// Resolves an `AuthentikGroup`'s Authentik-side identity by matching
    /// `name` via the Authentik API — the gateway never talks to `kube`,
    /// so this assumes the referencing CR's `spec.name` equals the
    /// referenced `AuthentikGroup` CR's `spec.name`, the convention this
    /// operator establishes for `parentRef`/`groupRefs`.
    async fn resolve_group_by_name(&self, name: &str) -> Result<uuid::Uuid, GatewayError> {
        let list = core_api::core_groups_list(
            &self.configuration,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(name),
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(map_err)?;
        list.results
            .into_iter()
            .find(|g| g.name == name)
            .map(|g| g.pk)
            .ok_or_else(|| GatewayError::NotFound(format!("group {name:?} not found")))
    }

    /// `outpost_ref` is `ProxyProviderSpec.outpost_ref` verbatim: `None`
    /// resolves to Authentik's embedded outpost, `Some` resolves an
    /// `AuthentikOutpost` by matching `name` (same caveat as
    /// `resolve_group_by_name` — no Kubernetes lookup here).
    async fn resolve_outpost(&self, outpost_ref: Option<&str>) -> Result<uuid::Uuid, GatewayError> {
        let name = outpost_ref.unwrap_or(EMBEDDED_OUTPOST_NAME);
        let list = outposts_api::outposts_instances_list(
            &self.configuration,
            None,
            None,
            None,
            Some(name),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(map_err)?;
        list.results
            .into_iter()
            .find(|o| o.name == name)
            .map(|o| o.pk)
            .ok_or_else(|| GatewayError::NotFound(format!("outpost {name:?} not found")))
    }

    fn redirect_uris(spec: &Oauth2ProviderSpec) -> Vec<models::RedirectUriRequest> {
        spec.allowed_redirect_uris
            .iter()
            .map(|r| models::RedirectUriRequest {
                matching_mode: match r.matching_mode {
                    MatchingMode::Strict => models::MatchingModeEnum::Strict,
                    MatchingMode::Regex => models::MatchingModeEnum::Regex,
                },
                url: r.url.clone(),
                redirect_uri_type: None,
            })
            .collect()
    }

    fn grant_types(
        spec: &Oauth2ProviderSpec,
    ) -> Result<Option<Vec<models::GrantTypesEnum>>, GatewayError> {
        if spec.grant_types.is_empty() {
            return Ok(None);
        }
        spec.grant_types
            .iter()
            .map(|g| match g.as_str() {
                "authorization_code" => Ok(models::GrantTypesEnum::AuthorizationCode),
                "implicit" => Ok(models::GrantTypesEnum::Implicit),
                "hybrid" => Ok(models::GrantTypesEnum::Hybrid),
                "refresh_token" => Ok(models::GrantTypesEnum::RefreshToken),
                "client_credentials" => Ok(models::GrantTypesEnum::ClientCredentials),
                "password" => Ok(models::GrantTypesEnum::Password),
                other => Err(GatewayError::Api(format!(
                    "unsupported oauth2 grant type {other:?}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn map_err<T>(err: authentik_client::apis::Error<T>) -> GatewayError {
    use authentik_client::apis::Error;
    if let Error::ResponseError(ref resp) = err {
        let status = resp.status.as_u16();
        if status == 400 || status == 409 {
            return GatewayError::AlreadyExists(resp.content.clone());
        }
        if status == 404 {
            return GatewayError::NotFound(resp.content.clone());
        }
    }
    GatewayError::Api(err.to_string())
}

fn parse_i32(id: &str) -> Result<i32, GatewayError> {
    id.parse()
        .map_err(|_| GatewayError::Api(format!("expected an integer id, got {id:?}")))
}

fn outpost_type(t: &OutpostType) -> models::OutpostTypeEnum {
    match t {
        OutpostType::Proxy => models::OutpostTypeEnum::Proxy,
        OutpostType::Ldap => models::OutpostTypeEnum::Ldap,
        OutpostType::Radius => models::OutpostTypeEnum::Radius,
    }
}

fn outpost_config(
    config: &serde_json::Value,
) -> std::collections::HashMap<String, serde_json::Value> {
    config
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect()
}

#[async_trait::async_trait]
impl AuthentikGateway for AuthentikHttpGateway {
    async fn create_application(
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

    async fn update_application(
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

    async fn delete_application(&self, authentik_id: &str) -> Result<(), GatewayError> {
        match core_api::core_applications_destroy(&self.configuration, authentik_id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    async fn get_application(&self, authentik_id: &str) -> Result<serde_json::Value, GatewayError> {
        let app = core_api::core_applications_retrieve(&self.configuration, authentik_id)
            .await
            .map_err(map_err)?;
        serde_json::to_value(app).map_err(|e| GatewayError::Api(e.to_string()))
    }

    async fn create_group(&self, group: &AuthentikGroup) -> Result<String, GatewayError> {
        let parents = match &group.spec.parent_ref {
            Some(name) => vec![self.resolve_group_by_name(name).await?],
            None => vec![],
        };
        let attributes = if group.spec.attributes.is_empty() {
            None
        } else {
            Some(
                group
                    .spec
                    .attributes
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect(),
            )
        };
        let req = models::GroupRequest {
            name: group.spec.name.clone(),
            is_superuser: Some(group.spec.is_superuser),
            parents: Some(parents),
            users: None,
            attributes,
            roles: None,
        };
        core_api::core_groups_create(&self.configuration, req)
            .await
            .map(|g| g.pk.to_string())
            .map_err(map_err)
    }

    async fn update_group(
        &self,
        authentik_id: &str,
        group: &AuthentikGroup,
    ) -> Result<(), GatewayError> {
        let parents = match &group.spec.parent_ref {
            Some(name) => vec![self.resolve_group_by_name(name).await?],
            None => vec![],
        };
        let attributes = Some(
            group
                .spec
                .attributes
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect(),
        );
        let req = models::PatchedGroupRequest {
            name: Some(group.spec.name.clone()),
            is_superuser: Some(group.spec.is_superuser),
            parents: Some(parents),
            users: None,
            attributes,
            roles: None,
        };
        core_api::core_groups_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    async fn delete_group(&self, authentik_id: &str) -> Result<(), GatewayError> {
        match core_api::core_groups_destroy(&self.configuration, authentik_id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    async fn create_user(&self, user: &AuthentikUser) -> Result<String, GatewayError> {
        let mut groups = Vec::with_capacity(user.spec.group_refs.len());
        for name in &user.spec.group_refs {
            groups.push(self.resolve_group_by_name(name).await?);
        }
        let req = models::UserRequest {
            username: user.spec.username.clone(),
            name: user.spec.name.clone(),
            is_active: Some(user.spec.is_active),
            last_login: None,
            groups: Some(groups),
            roles: None,
            email: Some(user.spec.email.clone()),
            attributes: None,
            path: None,
            r#type: None,
        };
        core_api::core_users_create(&self.configuration, req)
            .await
            .map(|u| u.pk.to_string())
            .map_err(map_err)
    }

    async fn update_user(
        &self,
        authentik_id: &str,
        user: &AuthentikUser,
    ) -> Result<(), GatewayError> {
        let id = parse_i32(authentik_id)?;
        let mut groups = Vec::with_capacity(user.spec.group_refs.len());
        for name in &user.spec.group_refs {
            groups.push(self.resolve_group_by_name(name).await?);
        }
        let req = models::PatchedUserRequest {
            username: Some(user.spec.username.clone()),
            name: Some(user.spec.name.clone()),
            is_active: Some(user.spec.is_active),
            last_login: None,
            groups: Some(groups),
            roles: None,
            email: Some(user.spec.email.clone()),
            attributes: None,
            path: None,
            r#type: None,
        };
        core_api::core_users_partial_update(&self.configuration, id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    async fn delete_user(&self, authentik_id: &str) -> Result<(), GatewayError> {
        let id = parse_i32(authentik_id)?;
        match core_api::core_users_destroy(&self.configuration, id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    async fn create_outpost(&self, outpost: &AuthentikOutpost) -> Result<String, GatewayError> {
        let req = models::OutpostRequest {
            name: outpost.spec.name.clone(),
            r#type: outpost_type(&outpost.spec.r#type),
            providers: vec![],
            service_connection: None,
            config: outpost_config(&outpost.spec.config),
            managed: None,
        };
        outposts_api::outposts_instances_create(&self.configuration, req)
            .await
            .map(|o| o.pk.to_string())
            .map_err(map_err)
    }

    async fn update_outpost(
        &self,
        authentik_id: &str,
        outpost: &AuthentikOutpost,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedOutpostRequest {
            name: Some(outpost.spec.name.clone()),
            r#type: Some(outpost_type(&outpost.spec.r#type)),
            providers: None,
            service_connection: None,
            config: Some(outpost_config(&outpost.spec.config)),
            managed: None,
        };
        outposts_api::outposts_instances_partial_update(
            &self.configuration,
            authentik_id,
            Some(req),
        )
        .await
        .map(|_| ())
        .map_err(map_err)
    }

    async fn delete_outpost(&self, authentik_id: &str) -> Result<(), GatewayError> {
        match outposts_api::outposts_instances_destroy(&self.configuration, authentik_id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    async fn attach_outpost(
        &self,
        outpost_ref: Option<&str>,
        provider_authentik_id: &str,
    ) -> Result<(), GatewayError> {
        let outpost_id = self.resolve_outpost(outpost_ref).await?.to_string();
        let provider_id = parse_i32(provider_authentik_id)?;

        let outpost = outposts_api::outposts_instances_retrieve(&self.configuration, &outpost_id)
            .await
            .map_err(map_err)?;

        let mut providers = outpost.providers;
        if !providers.contains(&provider_id) {
            providers.push(provider_id);
        }

        let patch = models::PatchedOutpostRequest {
            name: None,
            r#type: None,
            providers: Some(providers),
            service_connection: None,
            config: None,
            managed: None,
        };
        outposts_api::outposts_instances_partial_update(
            &self.configuration,
            &outpost_id,
            Some(patch),
        )
        .await
        .map(|_| ())
        .map_err(map_err)
    }

    async fn create_brand(&self, brand: &AuthentikBrand) -> Result<String, GatewayError> {
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

    async fn update_brand(
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

    async fn delete_brand(&self, authentik_id: &str) -> Result<(), GatewayError> {
        match core_api::core_brands_destroy(&self.configuration, authentik_id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    async fn get_default_brand(&self) -> Result<Option<String>, GatewayError> {
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

    async fn set_brand_default(
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

    async fn upsert_oauth2_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &Oauth2ProviderSpec,
    ) -> Result<Oauth2ProviderUpsertResult, GatewayError> {
        let authorization_flow = self.resolve_flow(&spec.authorization_flow).await?;
        let invalidation_flow = self.resolve_flow(&spec.invalidation_flow).await?;
        let property_mappings = self
            .resolve_property_mappings(&spec.property_mappings)
            .await?;
        let signing_key = match &spec.signing_key {
            Some(cert_name) => Some(self.resolve_certificate(cert_name).await?),
            None => None,
        };
        let redirect_uris = Self::redirect_uris(spec);
        let grant_types = Self::grant_types(spec)?;

        let provider = match authentik_id {
            None => {
                let req = models::OAuth2ProviderRequest {
                    name: name.to_string(),
                    authentication_flow: None,
                    authorization_flow,
                    invalidation_flow,
                    property_mappings: Some(property_mappings),
                    client_type: Some(models::ClientTypeEnum::Confidential),
                    grant_types,
                    client_id: spec.client_id.clone(),
                    client_secret: None,
                    access_code_validity: None,
                    access_token_validity: None,
                    refresh_token_validity: None,
                    refresh_token_threshold: None,
                    include_claims_in_id_token: None,
                    signing_key: signing_key.map(Some),
                    encryption_key: None,
                    redirect_uris,
                    logout_uri: None,
                    logout_method: None,
                    sub_mode: None,
                    issuer_mode: None,
                    jwt_federation_sources: None,
                    jwt_federation_providers: None,
                };
                providers_api::providers_oauth2_create(&self.configuration, req)
                    .await
                    .map_err(map_err)?
            }
            Some(id) => {
                let id = parse_i32(id)?;
                let req = models::PatchedOAuth2ProviderRequest {
                    name: Some(name.to_string()),
                    authentication_flow: None,
                    authorization_flow: Some(authorization_flow),
                    invalidation_flow: Some(invalidation_flow),
                    property_mappings: Some(property_mappings),
                    client_type: Some(models::ClientTypeEnum::Confidential),
                    grant_types,
                    client_id: spec.client_id.clone(),
                    client_secret: None,
                    access_code_validity: None,
                    access_token_validity: None,
                    refresh_token_validity: None,
                    refresh_token_threshold: None,
                    include_claims_in_id_token: None,
                    signing_key: signing_key.map(Some),
                    encryption_key: None,
                    redirect_uris: Some(redirect_uris),
                    logout_uri: None,
                    logout_method: None,
                    sub_mode: None,
                    issuer_mode: None,
                    jwt_federation_sources: None,
                    jwt_federation_providers: None,
                };
                providers_api::providers_oauth2_partial_update(&self.configuration, id, Some(req))
                    .await
                    .map_err(map_err)?
            }
        };

        Ok(Oauth2ProviderUpsertResult {
            authentik_id: provider.pk.to_string(),
            credentials: Oauth2Credentials {
                client_id: provider.client_id.unwrap_or_default(),
                client_secret: provider.client_secret.unwrap_or_default(),
                authentik_url: self.configuration.base_path.clone(),
            },
        })
    }

    async fn upsert_proxy_provider(
        &self,
        authentik_id: Option<&str>,
        name: &str,
        spec: &ProxyProviderSpec,
    ) -> Result<String, GatewayError> {
        let authorization_flow = self.resolve_flow(&spec.authorization_flow).await?;
        let invalidation_flow = self.resolve_flow(&spec.invalidation_flow).await?;

        let pk = match authentik_id {
            None => {
                let req = models::ProxyProviderRequest {
                    name: name.to_string(),
                    authentication_flow: None,
                    authorization_flow,
                    invalidation_flow,
                    property_mappings: None,
                    internal_host: Some(spec.internal_host.clone()),
                    external_host: spec.external_host.clone(),
                    internal_host_ssl_validation: None,
                    certificate: None,
                    skip_path_regex: None,
                    basic_auth_enabled: None,
                    basic_auth_password_attribute: None,
                    basic_auth_user_attribute: None,
                    mode: None,
                    intercept_header_auth: None,
                    cookie_domain: None,
                    jwt_federation_sources: None,
                    jwt_federation_providers: None,
                    access_token_validity: None,
                    refresh_token_validity: None,
                };
                providers_api::providers_proxy_create(&self.configuration, req)
                    .await
                    .map_err(map_err)?
                    .pk
            }
            Some(id) => {
                let id = parse_i32(id)?;
                let req = models::PatchedProxyProviderRequest {
                    name: Some(name.to_string()),
                    authentication_flow: None,
                    authorization_flow: Some(authorization_flow),
                    invalidation_flow: Some(invalidation_flow),
                    property_mappings: None,
                    internal_host: Some(spec.internal_host.clone()),
                    external_host: Some(spec.external_host.clone()),
                    internal_host_ssl_validation: None,
                    certificate: None,
                    skip_path_regex: None,
                    basic_auth_enabled: None,
                    basic_auth_password_attribute: None,
                    basic_auth_user_attribute: None,
                    mode: None,
                    intercept_header_auth: None,
                    cookie_domain: None,
                    jwt_federation_sources: None,
                    jwt_federation_providers: None,
                    access_token_validity: None,
                    refresh_token_validity: None,
                };
                providers_api::providers_proxy_partial_update(&self.configuration, id, Some(req))
                    .await
                    .map_err(map_err)?
                    .pk
            }
        };
        Ok(pk.to_string())
    }

    async fn upsert_policy_binding(
        &self,
        authentik_id: Option<&str>,
        application_authentik_id: &str,
        group_name: &str,
        order: i32,
        negate: bool,
    ) -> Result<String, GatewayError> {
        let target =
            core_api::core_applications_retrieve(&self.configuration, application_authentik_id)
                .await
                .map_err(map_err)?
                .pk;
        let group = self.resolve_group_by_name(group_name).await?;

        let pk = match authentik_id {
            None => {
                let req = models::PolicyBindingRequest {
                    policy: None,
                    group: Some(Some(group)),
                    user: None,
                    target,
                    negate: Some(negate),
                    enabled: Some(true),
                    order,
                    timeout: None,
                    failure_result: None,
                };
                policies_api::policies_bindings_create(&self.configuration, req)
                    .await
                    .map_err(map_err)?
                    .pk
            }
            Some(id) => {
                let req = models::PatchedPolicyBindingRequest {
                    policy: None,
                    group: Some(Some(group)),
                    user: None,
                    target: Some(target),
                    negate: Some(negate),
                    enabled: Some(true),
                    order: Some(order),
                    timeout: None,
                    failure_result: None,
                };
                policies_api::policies_bindings_partial_update(&self.configuration, id, Some(req))
                    .await
                    .map_err(map_err)?
                    .pk
            }
        };
        Ok(pk.to_string())
    }

    async fn delete_policy_binding(&self, authentik_id: &str) -> Result<(), GatewayError> {
        match policies_api::policies_bindings_destroy(&self.configuration, authentik_id).await {
            Ok(()) => Ok(()),
            Err(e) => match map_err(e) {
                GatewayError::NotFound(_) => Ok(()),
                other => Err(other),
            },
        }
    }
}
