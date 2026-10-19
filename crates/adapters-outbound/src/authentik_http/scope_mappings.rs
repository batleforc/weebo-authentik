use api::AuthentikScopeMapping;
use application::ports::GatewayError;
use authentik_client::apis::propertymappings_api;
use authentik_client::models;

use domain::drift::{Drift, same_trimmed};

use super::AuthentikHttpGateway;
use super::shared::{map_err, map_own_err};

impl AuthentikHttpGateway {
    pub(super) async fn create_scope_mapping_impl(
        &self,
        mapping: &AuthentikScopeMapping,
    ) -> Result<String, GatewayError> {
        let req = models::ScopeMappingRequest {
            // `managed` marks an object as owned by Authentik's own
            // migrations, which then overwrite it on upgrade. Ours is
            // owned by a CR, so it stays `None` — the same "unmanaged"
            // state a mapping created by hand in the UI would have.
            managed: None,
            name: mapping.spec.name.clone(),
            expression: mapping.spec.expression.clone(),
            scope_name: mapping.spec.scope_name.clone(),
            description: mapping.spec.description.clone(),
        };
        propertymappings_api::propertymappings_provider_scope_create(&self.configuration, req)
            .await
            .inspect(|_| self.created("scope_mapping"))
            .map(|m| m.pk.to_string())
            .map_err(map_err)
    }

    pub(super) async fn update_scope_mapping_impl(
        &self,
        authentik_id: &str,
        mapping: &AuthentikScopeMapping,
    ) -> Result<(), GatewayError> {
        // `description: None` here means "leave it alone", not "clear it"
        // — every other field is modeled, so this is the one place a
        // hand-edit in the UI survives a reconcile. Sending `Some("")`
        // instead would let the CR clear it, at the cost of every
        // description-less CR silently wiping one; matching the CRD's
        // `Option` to the patch's is the same choice `brands` makes for
        // its unmodeled fields.
        let remote = propertymappings_api::propertymappings_provider_scope_retrieve(
            &self.configuration,
            authentik_id,
        )
        .await
        .map_err(map_own_err)?;
        let mut req = models::PatchedScopeMappingRequest {
            managed: None,
            name: Some(mapping.spec.name.clone()),
            expression: Some(mapping.spec.expression.clone()),
            scope_name: Some(mapping.spec.scope_name.clone()),
            description: mapping.spec.description.clone(),
        };
        let mut drift = Drift::default();
        drift.keep("name", &mut req.name, &remote.name);
        // Authentik trims the text fields it stores: a block-scalar
        // expression (`|`, so ending in `\n`) never reads back as sent, and
        // compared byte for byte it drifted on every reconcile.
        drift.keep_with(
            "expression",
            &mut req.expression,
            &remote.expression,
            |d, r| same_trimmed(d, r),
        );
        drift.keep("scope_name", &mut req.scope_name, &remote.scope_name);
        drift.keep_with(
            "description",
            &mut req.description,
            &remote.description,
            |d, r| r.as_deref().is_some_and(|r| same_trimmed(d, r)),
        );
        if !self.must_write("scope_mapping", authentik_id, &drift) {
            return Ok(());
        }
        propertymappings_api::propertymappings_provider_scope_partial_update(
            &self.configuration,
            authentik_id,
            Some(req),
        )
        .await
        .map(|_| ())
        .map_err(map_own_err)
    }

    pub(super) async fn delete_scope_mapping_impl(
        &self,
        authentik_id: &str,
    ) -> Result<(), GatewayError> {
        self.gone(
            "scope_mapping",
            propertymappings_api::propertymappings_provider_scope_destroy(
                &self.configuration,
                authentik_id,
            )
            .await,
        )
    }
}
