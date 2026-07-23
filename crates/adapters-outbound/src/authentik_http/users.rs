use api::AuthentikUser;
use application::ports::GatewayError;
use authentik_client::apis::core_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err, parse_i32};

impl AuthentikHttpGateway {
    pub(super) async fn create_user_impl(
        &self,
        user: &AuthentikUser,
    ) -> Result<String, GatewayError> {
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

    pub(super) async fn update_user_impl(
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

    pub(super) async fn delete_user_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        let id = parse_i32(authentik_id)?;
        ignore_not_found(core_api::core_users_destroy(&self.configuration, id).await)
    }
}
