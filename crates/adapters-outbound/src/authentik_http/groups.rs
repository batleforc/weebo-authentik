use api::AuthentikGroup;
use application::ports::GatewayError;
use authentik_client::apis::core_api;
use authentik_client::models;

use domain::drift::{Drift, same_set};

use super::AuthentikHttpGateway;
use super::shared::{map_err, map_own_err};

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
            .inspect(|_| self.created("group"))
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
        // Without its members: only the group's own fields are compared.
        let remote = core_api::core_groups_retrieve(
            &self.configuration,
            authentik_id,
            Some(false), // include_children
            Some(false), // include_inherited_roles
            Some(false), // include_parents
            Some(false), // include_users
        )
        .await
        .map_err(map_own_err)?;
        let mut req = models::PatchedGroupRequest {
            name: Some(group.spec.name.clone()),
            is_superuser: Some(group.spec.is_superuser),
            parents: Some(parents),
            users: None,
            attributes,
            roles: None,
        };
        let mut drift = Drift::default();
        drift.keep("name", &mut req.name, &remote.name);
        drift.keep(
            "is_superuser",
            &mut req.is_superuser,
            &remote.is_superuser.unwrap_or(false),
        );
        drift.keep_with("parents", &mut req.parents, &remote.parents, |d, r| {
            same_set(d, r.as_deref().unwrap_or_default())
        });
        // The whole map is the operator's (it is sent whole): compared whole,
        // an absent one being empty.
        drift.keep_with(
            "attributes",
            &mut req.attributes,
            &remote.attributes,
            |d, r| r.as_ref().map_or(d.is_empty(), |r| d == r),
        );
        if !self.must_write("group", authentik_id, &drift) {
            return Ok(());
        }
        core_api::core_groups_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_own_err)
    }

    pub(super) async fn delete_group_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        self.gone(
            "group",
            core_api::core_groups_destroy(&self.configuration, authentik_id).await,
        )
    }
}
