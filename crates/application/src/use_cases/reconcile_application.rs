use api::AuthentikApplication;
use api::application::ProviderKind;
use domain::error::ReasonCode;

use crate::ports::{AuthentikGateway, GatewayError, SecretStore};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// Patch-first: diffs `spec` against the remote object read via
/// `authentik_id` and PATCHes; delete-recreate is reserved for a
/// `provider` variant change, gated behind the
/// `authentik.weebo.io/allow-disruptive-update` annotation (checked by the
/// controller adapter before calling this). See `.prompt/plan.md`,
/// "Politique de mutation".
///
/// Secret naming convention (previously open in `.prompt/plan.md`):
/// `SecretStore::write_oauth2_credentials` is called with the CR's own
/// `metadata.name`/`metadata.namespace` — one Secret per
/// `AuthentikApplication`, same name, same namespace as the CR that owns
/// it.
pub async fn reconcile_application(
    app: &AuthentikApplication,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
) -> ReconcileOutcome {
    if matches!(
        app.spec.provider.kind,
        ProviderKind::Saml | ProviderKind::Ldap
    ) {
        return ReconcileOutcome::Errored {
            reason: ReasonCode::UnsupportedProviderKind,
            message: format!(
                "provider kind {:?} is a schema-only stub, not implemented by any reconciler",
                app.spec.provider.kind
            ),
        };
    }

    // The CR's status never stores the provider's own id (only the
    // application's) — on update, the current provider FK is read back
    // off the live Authentik application so the right provider gets
    // patched in place rather than a new one created.
    let existing_provider_id: Option<String> = match authentik_id {
        Some(id) => match gateway.get_application(id).await {
            Ok(json) => json
                .get("provider")
                .and_then(serde_json::Value::as_i64)
                .map(|n| n.to_string()),
            Err(e) => return errored_from_gateway_error(e),
        },
        None => None,
    };

    let provider_id = match &app.spec.provider.kind {
        ProviderKind::Oauth2 => {
            let spec = app.spec.provider.oauth2.as_ref().expect(
                "kind=oauth2 implies oauth2 spec present, enforced by the reconciler contract",
            );
            let result = match gateway
                .upsert_oauth2_provider(existing_provider_id.as_deref(), &app.spec.name, spec)
                .await
            {
                Ok(result) => result,
                Err(e) => return errored_from_gateway_error(e),
            };

            let namespace = app.metadata.namespace.clone().unwrap_or_default();
            let name = app
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| app.spec.slug.clone());
            if let Err(e) = secrets
                .write_oauth2_credentials(&namespace, &name, &result.credentials)
                .await
            {
                return ReconcileOutcome::Errored {
                    reason: ReasonCode::SecretStoreError,
                    message: format!("failed writing oauth2 credentials secret: {e}"),
                };
            }

            match parse_provider_id(&result.authentik_id) {
                Ok(id) => id,
                Err(outcome) => return outcome,
            }
        }
        ProviderKind::Proxy => {
            let spec = app.spec.provider.proxy.as_ref().expect(
                "kind=proxy implies proxy spec present, enforced by the reconciler contract",
            );
            let id = match gateway
                .upsert_proxy_provider(existing_provider_id.as_deref(), &app.spec.name, spec)
                .await
            {
                Ok(id) => id,
                Err(e) => return errored_from_gateway_error(e),
            };
            match parse_provider_id(&id) {
                Ok(id) => id,
                Err(outcome) => return outcome,
            }
        }
        ProviderKind::Saml | ProviderKind::Ldap => unreachable!("rejected above"),
    };

    let app_result = match authentik_id {
        Some(id) => gateway
            .update_application(id, app, provider_id)
            .await
            .map(|()| id.to_string()),
        None => gateway.create_application(app, provider_id).await,
    };
    let new_authentik_id = match app_result {
        Ok(id) => id,
        Err(e) => return errored_from_gateway_error(e),
    };

    if let ProviderKind::Proxy = app.spec.provider.kind {
        let spec = app.spec.provider.proxy.as_ref().expect("checked above");
        if let Err(e) = gateway
            .attach_outpost(spec.outpost_ref.as_deref(), &provider_id.to_string())
            .await
        {
            return match e {
                GatewayError::NotFound(message) => ReconcileOutcome::Errored {
                    reason: ReasonCode::OutpostRefNotFound,
                    message,
                },
                other => errored_from_gateway_error(other),
            };
        }
    }

    ReconcileOutcome::Synced {
        authentik_id: Some(new_authentik_id),
    }
}

fn parse_provider_id(id: &str) -> Result<i32, ReconcileOutcome> {
    id.parse().map_err(|_| ReconcileOutcome::Errored {
        reason: ReasonCode::AuthentikApiError,
        message: format!("unexpected non-integer provider id {id:?}"),
    })
}
