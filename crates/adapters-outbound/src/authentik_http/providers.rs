use api::application::{
    MatchingMode, Oauth2ClientType, Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec,
};
use application::ports::{GatewayError, Oauth2Credentials, Oauth2ProviderUpsertResult};
use authentik_client::apis::{crypto_api, propertymappings_api, providers_api};
use authentik_client::models;

use domain::drift::{Drift, same_set};

use super::AuthentikHttpGateway;
use super::shared::{map_err, map_own_err, parse_i32};

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
                // Spelled as the URN, the way Authentik's own enum serializes
                // it, so a CR carries the value the API documents rather than a
                // short alias this operator would have to keep translating.
                "urn:ietf:params:oauth:grant-type:device_code" => Ok(
                    models::GrantTypesEnum::UrnColonIetfColonParamsColonOauthColonGrantTypeColonDeviceCode,
                ),
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
        slug: &str,
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
                    client_type: Some(match spec.client_type {
                        Oauth2ClientType::Public => models::ClientTypeEnum::Public,
                        Oauth2ClientType::Confidential => models::ClientTypeEnum::Confidential,
                    }),
                    grant_types,
                    client_id: spec.client_id.clone(),
                    client_secret: None,
                    access_code_validity: None,
                    access_token_validity: spec.access_token_validity.clone(),
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
                    .inspect(|_| self.created("oauth2_provider"))
                    .map_err(map_err)?
            }
            Some(id) => {
                let id = parse_i32(id)?;
                let remote = providers_api::providers_oauth2_retrieve(&self.configuration, id)
                    .await
                    .map_err(map_own_err)?;
                let mut req = models::PatchedOAuth2ProviderRequest {
                    name: Some(name.to_string()),
                    authentication_flow: None,
                    authorization_flow: Some(authorization_flow),
                    invalidation_flow: Some(invalidation_flow),
                    property_mappings: Some(property_mappings),
                    client_type: Some(match spec.client_type {
                        Oauth2ClientType::Public => models::ClientTypeEnum::Public,
                        Oauth2ClientType::Confidential => models::ClientTypeEnum::Confidential,
                    }),
                    grant_types,
                    client_id: spec.client_id.clone(),
                    client_secret: None,
                    access_code_validity: None,
                    access_token_validity: spec.access_token_validity.clone(),
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
                let mut drift = Drift::default();
                drift.keep("name", &mut req.name, &remote.name);
                drift.keep(
                    "authorization_flow",
                    &mut req.authorization_flow,
                    &remote.authorization_flow,
                );
                drift.keep(
                    "invalidation_flow",
                    &mut req.invalidation_flow,
                    &remote.invalidation_flow,
                );
                drift.keep_with(
                    "property_mappings",
                    &mut req.property_mappings,
                    &remote.property_mappings,
                    |d, r| same_set(d, r.as_deref().unwrap_or_default()),
                );
                drift.keep_opt("client_type", &mut req.client_type, &remote.client_type);
                drift.keep_with(
                    "grant_types",
                    &mut req.grant_types,
                    &remote.grant_types,
                    |d, r| same_set(d, r.as_deref().unwrap_or_default()),
                );
                drift.keep_opt("client_id", &mut req.client_id, &remote.client_id);
                drift.keep_opt(
                    "access_token_validity",
                    &mut req.access_token_validity,
                    &remote.access_token_validity,
                );
                drift.keep_opt("signing_key", &mut req.signing_key, &remote.signing_key);
                // Two types (request vs response) for the same pair: compared
                // as a set of (matching mode, URL).
                drift.keep_with(
                    "redirect_uris",
                    &mut req.redirect_uris,
                    &remote.redirect_uris,
                    |d, r| {
                        let d: Vec<_> = d
                            .iter()
                            .map(|u| (u.matching_mode, u.url.as_str()))
                            .collect();
                        let r: Vec<_> = r
                            .iter()
                            .map(|u| (u.matching_mode, u.url.as_str()))
                            .collect();
                        same_set(&d, &r)
                    },
                );
                if self.must_write("oauth2_provider", &id.to_string(), &drift) {
                    providers_api::providers_oauth2_partial_update(
                        &self.configuration,
                        id,
                        Some(req),
                    )
                    .await
                    .map_err(map_own_err)?
                } else {
                    // In sync: the credentials are the ones just read.
                    remote
                }
            }
        };

        // Credentials missing from the response would be written as is and
        // the CR reported Synced with an unusable secret: only a public
        // client may have no secret.
        let client_id = provider
            .client_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                GatewayError::Api(format!(
                    "oauth2 provider {} came back without a client_id",
                    provider.pk
                ))
            })?;
        let client_secret = match (
            provider.client_secret.filter(|s| !s.is_empty()),
            &spec.client_type,
        ) {
            (Some(secret), _) => secret,
            (None, Oauth2ClientType::Public) => String::new(),
            (None, Oauth2ClientType::Confidential) => {
                return Err(GatewayError::Api(format!(
                    "confidential oauth2 provider {} came back without a client_secret",
                    provider.pk
                )));
            }
        };
        Ok(Oauth2ProviderUpsertResult {
            authentik_id: provider.pk.to_string(),
            credentials: Oauth2Credentials {
                client_id,
                client_secret,
                // The per-application OIDC issuer, NOT the API base path.
                // Authentik issues tokens under
                // `<web_base>/application/o/<application_slug>/` (the
                // default `per_provider` issuer mode); consumers
                // (e.g. argocd's `oidc.config` issuer, an app's
                // `OIDC_ISSUER_URL`) resolve discovery from this exact URL.
                // Previously this wrote `configuration.base_path`, which is
                // the `/api/v3` REST endpoint — wrong for every consumer.
                authentik_url: format!("{}/application/o/{slug}/", self.web_base_url),
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
                    .inspect(|_| self.created("proxy_provider"))
                    .map_err(map_err)?
                    .pk
            }
            Some(id) => {
                let id = parse_i32(id)?;
                let remote = providers_api::providers_proxy_retrieve(&self.configuration, id)
                    .await
                    .map_err(map_own_err)?;
                let mut req = models::PatchedProxyProviderRequest {
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
                let mut drift = Drift::default();
                drift.keep("name", &mut req.name, &remote.name);
                drift.keep(
                    "authorization_flow",
                    &mut req.authorization_flow,
                    &remote.authorization_flow,
                );
                drift.keep(
                    "invalidation_flow",
                    &mut req.invalidation_flow,
                    &remote.invalidation_flow,
                );
                drift.keep_opt(
                    "internal_host",
                    &mut req.internal_host,
                    &remote.internal_host,
                );
                drift.keep(
                    "external_host",
                    &mut req.external_host,
                    &remote.external_host,
                );
                // Each save of a proxy provider makes Authentik rebuild its
                // outpost permissions and push the outpost an update: the
                // write avoided here is the costliest of the operator.
                if !self.must_write("proxy_provider", &id.to_string(), &drift) {
                    return Ok(remote.pk.to_string());
                }
                providers_api::providers_proxy_partial_update(&self.configuration, id, Some(req))
                    .await
                    .map_err(map_own_err)?
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
            Ok(()) => {
                self.wrote_deleted("provider");
                Ok(())
            }
            Err(GatewayError::NotFound(_)) => Ok(()),
            Err(other) => Err(other),
        }
    }

    pub(super) async fn delete_any_provider_impl(
        &self,
        authentik_id: i32,
    ) -> Result<(), GatewayError> {
        self.gone(
            "provider",
            providers_api::providers_all_destroy(&self.configuration, authentik_id).await,
        )
    }
}
