use application::ports::GatewayError;
use authentik_client::apis::core_api;

use super::AuthentikHttpGateway;

pub(super) fn map_err<T>(err: authentik_client::apis::Error<T>) -> GatewayError {
    use authentik_client::apis::Error;
    if let Error::ResponseError(ref resp) = err {
        let status = resp.status.as_u16();
        if status == 409 || (status == 400 && is_duplicate_conflict(&resp.content)) {
            return GatewayError::AlreadyExists(resp.content.clone());
        }
        if status == 404 {
            return GatewayError::NotFound(resp.content.clone());
        }
    }
    GatewayError::Api(err.to_string())
}

/// Shared by every `delete_*` method: a 404 on delete means the object is
/// already gone, which is success (idempotent delete), not a failure —
/// same intent as `K8sSecretStore::delete`'s treatment of a 404 from the
/// Kubernetes API.
pub(super) fn ignore_not_found<T>(
    result: Result<(), authentik_client::apis::Error<T>>,
) -> Result<(), GatewayError> {
    match result.map_err(map_err) {
        Ok(()) => Ok(()),
        Err(GatewayError::NotFound(_)) => Ok(()),
        Err(other) => Err(other),
    }
}

/// Authentik reports a name/slug collision as an HTTP 400 (via Django REST
/// Framework's default `UniqueValidator`) whose body reads `{"<field>":
/// ["<Model> with this <field> already exists."]}` — confirmed live
/// against a real instance for both groups ("Group with this name already
/// exists.") and applications ("Application with this slug already
/// exists."). A 400 from any other validation failure (missing/invalid
/// field, unresolvable flow reference, etc.) never contains this phrase —
/// confirmed the same way — so this is how a real conflict is told apart
/// from an unrelated validation error that would otherwise be misreported
/// as `AlreadyExists`.
fn is_duplicate_conflict(body: &str) -> bool {
    body.to_ascii_lowercase().contains("already exists")
}

pub(super) fn parse_i32(id: &str) -> Result<i32, GatewayError> {
    id.parse()
        .map_err(|_| GatewayError::Api(format!("expected an integer id, got {id:?}")))
}

impl AuthentikHttpGateway {
    /// Flows are referenced by slug (no `AuthentikFlow` CRD in v1, see
    /// `.prompt/plan.md`) — resolved to a pk directly against the
    /// Authentik API on every reconcile. Used by both `brands` (via
    /// `resolve_optional_flow`) and `providers`.
    pub(super) async fn resolve_flow(&self, slug: &str) -> Result<uuid::Uuid, GatewayError> {
        let list = authentik_client::apis::flows_api::flows_instances_list(
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

    /// Resolves an `AuthentikGroup`'s Authentik-side identity by matching
    /// `name` via the Authentik API — the gateway never talks to `kube`,
    /// so this assumes the referencing CR's `spec.name` equals the
    /// referenced `AuthentikGroup` CR's `spec.name`, the convention this
    /// operator establishes for `parentRef`/`groupRefs`. Used by `groups`,
    /// `users`, and `policy_bindings`.
    pub(super) async fn resolve_group_by_name(
        &self,
        name: &str,
    ) -> Result<uuid::Uuid, GatewayError> {
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
}
