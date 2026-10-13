use application::ports::GatewayError;
use authentik_client::apis::{core_api, policies_api};
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err, map_own_err};

impl AuthentikHttpGateway {
    pub(super) async fn upsert_policy_binding_impl(
        &self,
        authentik_id: Option<&str>,
        application_authentik_id: &str,
        group_name: &str,
        order: i32,
        negate: bool,
    ) -> Result<String, GatewayError> {
        // The application this binding targets, by the slug its CR
        // recorded: a 404 is neither the binding gone nor its group ref.
        let target =
            core_api::core_applications_retrieve(&self.configuration, application_authentik_id)
                .await
                .map_err(|e| match map_err(e) {
                    GatewayError::NotFound(_) => GatewayError::Api(format!(
                        "application {application_authentik_id:?} not found in Authentik"
                    )),
                    other => other,
                })?
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
                    .map_err(map_own_err)?
                    .pk
            }
        };
        Ok(pk.to_string())
    }

    pub(super) async fn delete_policy_binding_impl(
        &self,
        authentik_id: &str,
    ) -> Result<(), GatewayError> {
        ignore_not_found(
            policies_api::policies_bindings_destroy(&self.configuration, authentik_id).await,
        )
    }
}
