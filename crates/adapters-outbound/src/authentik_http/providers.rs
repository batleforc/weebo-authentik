use api::application::{MatchingMode, Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec};
use application::ports::{GatewayError, Oauth2Credentials, Oauth2ProviderUpsertResult};
use authentik_client::apis::{crypto_api, propertymappings_api, providers_api};
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{map_err, parse_i32};

impl AuthentikHttpGateway {
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

    pub(super) async fn upsert_oauth2_provider_impl(
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

    pub(super) async fn upsert_proxy_provider_impl(
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

    pub(super) async fn delete_provider_impl(
        &self,
        authentik_id: i32,
        kind: ProviderKind,
    ) -> Result<(), GatewayError> {
        let result = match kind {
            ProviderKind::Oauth2 => {
                providers_api::providers_oauth2_destroy(&self.configuration, authentik_id)
                    .await
                    .map_err(map_err)
            }
            ProviderKind::Proxy => {
                providers_api::providers_proxy_destroy(&self.configuration, authentik_id)
                    .await
                    .map_err(map_err)
            }
            ProviderKind::Saml | ProviderKind::Ldap => {
                return Err(GatewayError::Api(format!(
                    "delete_provider called with unsupported kind {kind:?} — only oauth2/proxy \
                     providers are ever created by this operator"
                )));
            }
        };
        match result {
            Ok(()) => Ok(()),
            Err(GatewayError::NotFound(_)) => Ok(()),
            Err(other) => Err(other),
        }
    }
}
