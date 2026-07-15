use api::AuthentikUser;

use crate::ports::AuthentikGateway;
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// Note the CR carries no credential field by design (see `api::user`) so
/// this use-case never touches a password.
pub async fn reconcile_user(
    user: &AuthentikUser,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let result = match authentik_id {
        Some(id) => gateway.update_user(id, user).await.map(|()| id.to_string()),
        None => gateway.create_user(user).await,
    };

    match result {
        Ok(id) => ReconcileOutcome::Synced {
            authentik_id: Some(id),
        },
        Err(e) => errored_from_gateway_error(e),
    }
}
