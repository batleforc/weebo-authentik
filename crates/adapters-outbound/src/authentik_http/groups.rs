use api::AuthentikGroup;
use application::ports::GatewayError;
use authentik_client::apis::core_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err};

impl AuthentikHttpGateway {
    pub(super) async fn create_group_impl(
        &self,
        group: &AuthentikGroup,
    ) -> Result<String, GatewayError> {
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

    pub(super) async fn update_group_impl(
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

    pub(super) async fn delete_group_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        ignore_not_found(core_api::core_groups_destroy(&self.configuration, authentik_id).await)
    }
}
