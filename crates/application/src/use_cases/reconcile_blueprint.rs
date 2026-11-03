use api::AuthentikBlueprint;
use domain::error::ReasonCode;

use crate::ports::{AuthentikGateway, BlueprintApplyStatus, GatewayError};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

/// What a reconcile of a blueprint leaves to record: its outcome, and the
/// instance it knows to exist — recorded even when the apply that followed
/// is still pending or failed, so the next attempt patches the instance
/// instead of creating it again (and colliding with its unique name).
#[derive(Debug)]
pub struct BlueprintReconciled {
    pub outcome: ReconcileOutcome,
    /// The instance (`status.authentikId`), once created or updated by this
    /// reconcile; `None` leaves the recorded one as it was.
    pub authentik_id: Option<String>,
}

/// Create-then-update keyed on `status.authentikId` (same shape as
/// `reconcile_scope_mapping`), then the apply:
///
/// 1. the instance is written — Authentik validates the content there, so
///    an invalid blueprint stops here with `BlueprintInvalid`;
/// 2. disabled (`spec.enabled: false`): nothing to apply, `Synced`;
/// 3. the content Authentik stores was applied successfully: `Synced`;
/// 4. otherwise an apply is asked for, and the outcome is
///    `BlueprintApplyFailed` when the last one failed, else
///    `BlueprintApplyPending` — both retried shortly, which is how the
///    asynchronous result is picked up.
///
/// Step 4 asks again on every retry until the content is applied: a queued
/// apply that has not run yet may be asked for twice, which only applies
/// the same content twice.
pub async fn reconcile_blueprint(
    blueprint: &AuthentikBlueprint,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
) -> BlueprintReconciled {
    let written = match authentik_id {
        // Deleted out of band: recreated, the CR is authoritative.
        Some(id) => match gateway.update_blueprint(id, blueprint).await {
            Err(GatewayError::Gone(_)) => gateway.create_blueprint(blueprint).await,
            result => result.map(|()| id.to_string()),
        },
        None => gateway.create_blueprint(blueprint).await,
    };
    let id = match written {
        Ok(id) => id,
        Err(e) => {
            return BlueprintReconciled {
                outcome: errored(e),
                authentik_id: None,
            };
        }
    };
    let outcome = apply_outcome(blueprint, &id, gateway).await;
    BlueprintReconciled {
        outcome,
        authentik_id: Some(id),
    }
}

async fn apply_outcome(
    blueprint: &AuthentikBlueprint,
    id: &str,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    let synced = ReconcileOutcome::Synced {
        authentik_id: Some(id.to_string()),
    };
    if !blueprint.spec.is_enabled() {
        return synced;
    }
    let state = match gateway.get_blueprint_state(id).await {
        Ok(state) => state,
        Err(e) => return errored(e),
    };
    if state.content_applied && state.status == BlueprintApplyStatus::Successful {
        return synced;
    }
    if let Err(e) = gateway.apply_blueprint(id).await {
        return errored(e);
    }
    match state.status {
        BlueprintApplyStatus::Failed => ReconcileOutcome::Errored {
            reason: ReasonCode::BlueprintApplyFailed,
            message: "Authentik's worker failed to apply this blueprint (see its task log, \
                      System > Blueprints); applying it again"
                .to_string(),
        },
        BlueprintApplyStatus::Successful | BlueprintApplyStatus::Unknown => {
            ReconcileOutcome::Errored {
                reason: ReasonCode::BlueprintApplyPending,
                message: "content stored, apply queued on Authentik's worker".to_string(),
            }
        }
    }
}

/// `Invalid` is the content refused by Authentik's importer: its own
/// reason, the validation log as the message. Anything else is the shared
/// mapping.
fn errored(err: GatewayError) -> ReconcileOutcome {
    match err {
        GatewayError::Invalid(message) => ReconcileOutcome::Errored {
            reason: ReasonCode::BlueprintInvalid,
            message,
        },
        other => errored_from_gateway_error(other),
    }
}

#[cfg(test)]
mod tests {
    use api::blueprint::AuthentikBlueprintSpec;
    use domain::error::ReasonCode;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::{BlueprintState, GatewayError};
    use crate::test_support::FakeGateway;

    const PK: &str = "6f1c3b9e-0000-4000-8000-000000000001";

    fn blueprint(enabled: Option<bool>) -> AuthentikBlueprint {
        AuthentikBlueprint {
            metadata: ObjectMeta {
                name: Some("admin-step-up".to_string()),
                ..Default::default()
            },
            spec: AuthentikBlueprintSpec {
                name: "weebo - admin step-up".to_string(),
                content: "version: 1\nentries: []\n".to_string(),
                context: None,
                enabled,
            },
            status: None,
        }
    }

    fn state(status: BlueprintApplyStatus, content_applied: bool) -> BlueprintState {
        BlueprintState {
            status,
            content_applied,
        }
    }

    fn reason(outcome: &ReconcileOutcome) -> Option<ReasonCode> {
        match outcome {
            ReconcileOutcome::Errored { reason, .. } => Some(*reason),
            ReconcileOutcome::Synced { .. } => None,
        }
    }

    #[tokio::test]
    async fn a_new_blueprint_is_created_applied_and_pending() {
        let gateway = FakeGateway::create(Ok(PK.to_string()))
            .with_blueprint_state(Ok(state(BlueprintApplyStatus::Unknown, false)));
        let r = reconcile_blueprint(&blueprint(None), None, &gateway).await;
        assert_eq!(r.authentik_id.as_deref(), Some(PK));
        assert_eq!(reason(&r.outcome), Some(ReasonCode::BlueprintApplyPending));
        assert_eq!(gateway.applied(), vec![PK.to_string()]);
    }

    #[tokio::test]
    async fn applied_content_is_synced_without_a_new_apply() {
        let gateway = FakeGateway::update(Ok(()))
            .with_blueprint_state(Ok(state(BlueprintApplyStatus::Successful, true)));
        let r = reconcile_blueprint(&blueprint(None), Some(PK), &gateway).await;
        assert!(matches!(
            r.outcome,
            ReconcileOutcome::Synced { authentik_id: Some(ref id) } if id == PK
        ));
        assert!(gateway.applied().is_empty());
    }

    /// A content change: Authentik still reports the previous apply as
    /// successful, but for other content.
    #[tokio::test]
    async fn changed_content_is_applied_again() {
        let gateway = FakeGateway::update(Ok(()))
            .with_blueprint_state(Ok(state(BlueprintApplyStatus::Successful, false)));
        let r = reconcile_blueprint(&blueprint(None), Some(PK), &gateway).await;
        assert_eq!(reason(&r.outcome), Some(ReasonCode::BlueprintApplyPending));
        assert_eq!(gateway.applied(), vec![PK.to_string()]);
    }

    #[tokio::test]
    async fn a_failed_apply_is_reported_and_asked_again() {
        let gateway = FakeGateway::update(Ok(()))
            .with_blueprint_state(Ok(state(BlueprintApplyStatus::Failed, false)));
        let r = reconcile_blueprint(&blueprint(None), Some(PK), &gateway).await;
        assert_eq!(reason(&r.outcome), Some(ReasonCode::BlueprintApplyFailed));
        assert_eq!(r.authentik_id.as_deref(), Some(PK));
        assert_eq!(gateway.applied(), vec![PK.to_string()]);
    }

    /// A later apply of the same content failed (Authentik keeps the hash
    /// of the last success): not synced.
    #[tokio::test]
    async fn a_failure_after_a_success_is_not_synced() {
        let gateway = FakeGateway::update(Ok(()))
            .with_blueprint_state(Ok(state(BlueprintApplyStatus::Failed, true)));
        let r = reconcile_blueprint(&blueprint(None), Some(PK), &gateway).await;
        assert_eq!(reason(&r.outcome), Some(ReasonCode::BlueprintApplyFailed));
    }

    #[tokio::test]
    async fn a_disabled_blueprint_is_written_but_never_applied() {
        let gateway = FakeGateway::update(Ok(()));
        let r = reconcile_blueprint(&blueprint(Some(false)), Some(PK), &gateway).await;
        assert!(matches!(r.outcome, ReconcileOutcome::Synced { .. }));
        assert!(gateway.applied().is_empty());
    }

    #[tokio::test]
    async fn invalid_content_has_its_own_reason_and_records_nothing() {
        let gateway = FakeGateway::create(Err(GatewayError::Invalid(
            "Entry invalid: ... flow not found".to_string(),
        )));
        let r = reconcile_blueprint(&blueprint(None), None, &gateway).await;
        assert_eq!(reason(&r.outcome), Some(ReasonCode::BlueprintInvalid));
        assert!(r.authentik_id.is_none());
    }

    #[tokio::test]
    async fn a_taken_name_is_already_exists() {
        let gateway = FakeGateway::create(Err(GatewayError::AlreadyExists("dup".to_string())));
        let r = reconcile_blueprint(&blueprint(None), None, &gateway).await;
        assert_eq!(
            reason(&r.outcome),
            Some(ReasonCode::AuthentikObjectAlreadyExists)
        );
    }

    #[tokio::test]
    async fn an_instance_deleted_out_of_band_is_recreated() {
        let gateway = FakeGateway {
            update_result: Some(Err(GatewayError::Gone("gone".to_string()))),
            create_result: Some(Ok("new-pk".to_string())),
            ..Default::default()
        }
        .with_blueprint_state(Ok(state(BlueprintApplyStatus::Unknown, false)));
        let r = reconcile_blueprint(&blueprint(None), Some(PK), &gateway).await;
        assert_eq!(r.authentik_id.as_deref(), Some("new-pk"));
        assert_eq!(gateway.applied(), vec!["new-pk".to_string()]);
    }

    /// The instance exists even though reading its state failed: still
    /// recorded, so the retry patches it.
    #[tokio::test]
    async fn the_id_is_recorded_when_reading_the_state_fails() {
        let gateway = FakeGateway::create(Ok(PK.to_string()))
            .with_blueprint_state(Err(GatewayError::Api("boom".to_string())));
        let r = reconcile_blueprint(&blueprint(None), None, &gateway).await;
        assert_eq!(r.authentik_id.as_deref(), Some(PK));
        assert_eq!(reason(&r.outcome), Some(ReasonCode::AuthentikApiError));
    }
}
