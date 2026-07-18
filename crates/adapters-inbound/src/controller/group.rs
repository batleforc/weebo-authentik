use std::sync::Arc;

use api::AuthentikGroup;
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_group::reconcile_group;
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, FINALIZER, patch_ready_condition, patch_synced_status};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    #[error("finalizer error: {0}")]
    Finalizer(String),
    #[error("authentik gateway error: {0}")]
    Gateway(String),
}

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikGroup> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikGroup reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikGroup>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikGroup> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(group) => apply(&api, &group, &ctx).await,
            FinalizerEvent::Cleanup(group) => cleanup(&api, &group, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikGroup>,
    group: &AuthentikGroup,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = group.name_any();
    // Re-fetched directly from the API server rather than trusted from
    // `group` (sourced from kube-runtime's local watch cache): reconciles
    // for one object are serialized, but the watch event carrying a
    // *previous* reconcile's `patch_synced_status` can still be in flight
    // when this one starts (e.g. right behind the finalizer-add patch),
    // so a cached `authentik_id: None` here would be stale and trigger a
    // spurious second `create_group` — which the gateway can never
    // recover from (see `ports.rs`, "never a silent adopt"). Confirmed in
    // practice: without this, the finalizer-add-triggered reconcile that
    // immediately follows a successful create still reads a stale
    // `None` and re-creates, colliding with the object it just made.
    let current = api.get(&name).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_group(group, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikGroup", started, &outcome);

    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => {
            patch_synced_status(api, &name, &id, "group synced").await?;
        }
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "group synced",
            )
            .await?;
        }
        ReconcileOutcome::Errored { reason, message } => {
            patch_ready_condition(api, &name, ConditionStatus::False, reason, message).await?;
        }
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

async fn cleanup(
    _api: &Api<AuthentikGroup>,
    group: &AuthentikGroup,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = group
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_group(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    Ok(Action::await_change())
}

fn error_policy(_obj: Arc<AuthentikGroup>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
