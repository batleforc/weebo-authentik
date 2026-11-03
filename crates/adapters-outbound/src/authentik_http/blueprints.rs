use std::collections::HashMap;

use api::AuthentikBlueprint;
use application::ports::{BlueprintApplyStatus, BlueprintState, GatewayError};
use authentik_client::apis::{Error as ClientError, managed_api};
use authentik_client::models;
use domain::drift::{Drift, same_trimmed};
use sha2::{Digest, Sha512};

use super::AuthentikHttpGateway;
use super::shared::{map_err, map_own_err};

impl AuthentikHttpGateway {
    pub(super) async fn create_blueprint_impl(
        &self,
        blueprint: &AuthentikBlueprint,
    ) -> Result<String, GatewayError> {
        let req = models::BlueprintInstanceRequest {
            name: blueprint.spec.name.clone(),
            // Inline content, never a file: `path` names a file under the
            // worker's blueprints directory, which is exactly what this
            // kind replaces.
            path: None,
            context: context_of(blueprint)?,
            enabled: Some(blueprint.spec.is_enabled()),
            content: Some(blueprint.spec.content.clone()),
        };
        managed_api::managed_blueprints_create(&self.configuration, req)
            .await
            .inspect(|_| self.created("blueprint"))
            .map(|b| b.pk.to_string())
            .map_err(|e| invalid_or(e, map_err))
    }

    pub(super) async fn update_blueprint_impl(
        &self,
        authentik_id: &str,
        blueprint: &AuthentikBlueprint,
    ) -> Result<(), GatewayError> {
        let remote = managed_api::managed_blueprints_retrieve(&self.configuration, authentik_id)
            .await
            .map_err(map_own_err)?;
        // `context: None` leaves the stored context alone, like an unset
        // `description` on a scope mapping: a CR that never mentions it
        // does not wipe one set in the UI.
        let mut req = models::PatchedBlueprintInstanceRequest {
            name: Some(blueprint.spec.name.clone()),
            path: None,
            context: context_of(blueprint)?,
            enabled: Some(blueprint.spec.is_enabled()),
            content: Some(blueprint.spec.content.clone()),
        };
        let mut drift = Drift::default();
        drift.keep("name", &mut req.name, &remote.name);
        // Authentik trims what it stores (DRF `CharField`): a block-scalar
        // content ends in `\n` that never reads back.
        drift.keep_with("content", &mut req.content, &remote.content, |d, r| {
            same_trimmed(d, r.as_deref().unwrap_or_default())
        });
        drift.keep_opt("context", &mut req.context, &remote.context);
        // Authentik leaves `enabled` unset on an instance created without
        // it; unset means enabled there too.
        drift.keep_with("enabled", &mut req.enabled, &remote.enabled, |d, r| {
            *d == r.unwrap_or(true)
        });
        if !self.must_write("blueprint", authentik_id, &drift) {
            return Ok(());
        }
        managed_api::managed_blueprints_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(|e| invalid_or(e, map_own_err))
    }

    pub(super) async fn get_blueprint_state_impl(
        &self,
        authentik_id: &str,
    ) -> Result<BlueprintState, GatewayError> {
        let remote = managed_api::managed_blueprints_retrieve(&self.configuration, authentik_id)
            .await
            .map_err(map_own_err)?;
        Ok(state_of(&remote))
    }

    pub(super) async fn apply_blueprint_impl(
        &self,
        authentik_id: &str,
    ) -> Result<(), GatewayError> {
        tracing::info!(authentik_id, "blueprint apply requested");
        managed_api::managed_blueprints_apply_create(&self.configuration, authentik_id)
            .await
            .map(|_| ())
            .map_err(map_own_err)
    }

    pub(super) async fn delete_blueprint_impl(
        &self,
        authentik_id: &str,
    ) -> Result<(), GatewayError> {
        self.gone(
            "blueprint",
            managed_api::managed_blueprints_destroy(&self.configuration, authentik_id).await,
        )
    }
}

/// `spec.context` as the API's map. The CRD types it as an object; a CR
/// written around the schema (another type) is refused here rather than
/// sent.
fn context_of(
    blueprint: &AuthentikBlueprint,
) -> Result<Option<HashMap<String, serde_json::Value>>, GatewayError> {
    match &blueprint.spec.context {
        None => Ok(None),
        Some(serde_json::Value::Object(map)) => Ok(Some(
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        )),
        Some(other) => Err(GatewayError::Invalid(format!(
            "spec.context must be an object, got {other}"
        ))),
    }
}

/// What the last apply left. `last_applied_hash` is the SHA-512 (hex) of
/// the content the worker applied — `apply_blueprint` hashes
/// `instance.retrieve()`, the content as stored — so the stored content
/// is what it is compared with.
pub(super) fn state_of(remote: &models::BlueprintInstance) -> BlueprintState {
    use models::BlueprintInstanceStatusEnum as S;
    let status = match remote.status {
        S::Successful | S::Warning => BlueprintApplyStatus::Successful,
        S::Error | S::Orphaned => BlueprintApplyStatus::Failed,
        S::Unknown => BlueprintApplyStatus::Unknown,
    };
    let stored = remote.content.as_deref().unwrap_or_default();
    let content_applied =
        format!("{:x}", Sha512::digest(stored.as_bytes())) == remote.last_applied_hash;
    BlueprintState {
        status,
        content_applied,
    }
}

/// A 400 whose body names `content` (or `context`) is Authentik's
/// importer refusing the blueprint: `GatewayError::Invalid`, with its
/// validation log, rather than whatever `fallback` makes of it. Checked
/// before the fallback on purpose: that log can itself read "already
/// exists" (an entry colliding with an object), which `map_err` would take
/// for a name collision of the instance.
fn invalid_or<T>(
    err: ClientError<T>,
    fallback: impl FnOnce(ClientError<T>) -> GatewayError,
) -> GatewayError {
    if let ClientError::ResponseError(ref resp) = err
        && resp.status.as_u16() == 400
        && let Some(log) = validation_log(&resp.content)
    {
        return GatewayError::Invalid(log);
    }
    fallback(err)
}

/// The `content`/`context` messages of a DRF 400 body, one per line —
/// `None` when the body is not about either field.
pub(super) fn validation_log(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let lines: Vec<String> = ["content", "context"]
        .iter()
        .filter_map(|field| value.get(field))
        .flat_map(|messages| match messages {
            serde_json::Value::Array(items) => items
                .iter()
                .map(|m| m.as_str().map_or_else(|| m.to_string(), str::to_string))
                .collect(),
            other => vec![
                other
                    .as_str()
                    .map_or_else(|| other.to_string(), str::to_string),
            ],
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_error_is_a_validation_log() {
        let body =
            r#"{"content":["Failed to validate blueprint","- Invalid blueprint: flow not found"]}"#;
        assert_eq!(
            validation_log(body).as_deref(),
            Some("Failed to validate blueprint\n- Invalid blueprint: flow not found")
        );
    }

    #[test]
    fn a_name_collision_is_not_a_validation_log() {
        let body = r#"{"name":["blueprint instance with this name already exists."]}"#;
        assert_eq!(validation_log(body), None);
    }

    #[test]
    fn a_non_json_body_is_not_a_validation_log() {
        assert_eq!(validation_log("<html>"), None);
    }
}
