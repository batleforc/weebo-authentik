use std::sync::Arc;

use api::AuthentikBlueprint;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_blueprint::{BlueprintReconciled, reconcile_blueprint};
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikBlueprint>(client.clone());
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikBlueprint", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikBlueprint>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikBlueprint> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(blueprint) => apply(&api, &blueprint, &ctx).await,
            FinalizerEvent::Cleanup(blueprint) => {
                super::finalize(blueprint.as_ref(), &ctx, cleanup(&blueprint, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikBlueprint>,
    blueprint: &AuthentikBlueprint,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // Fresh from the API server, not the watch cache: see `super::current`.
    let current = super::current(api, blueprint).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let BlueprintReconciled {
        outcome,
        authentik_id: written_id,
    } = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => {
            let reconciled =
                reconcile_blueprint(blueprint, authentik_id.as_deref(), gateway.as_ref()).await;
            super::report_writes(
                "AuthentikBlueprint",
                &ctx.recorder,
                &current,
                gateway.as_ref(),
            )
            .await;
            reconciled
        }
        Err(e) => BlueprintReconciled {
            outcome: errored_from_factory_error(e),
            authentik_id: None,
        },
    };
    // The instance is recorded as soon as it exists in Authentik, even
    // while its apply is pending or failed: the next attempt patches it
    // instead of colliding with its unique name, and the finalizer deletes
    // it.
    let mut fields = serde_json::Map::new();
    if let Some(id) = written_id {
        fields.insert("authentikId".into(), id.into());
    }
    super::record_reconcile("AuthentikBlueprint", started, &outcome);
    // Pending and failed applies are not `needs_a_change` reasons: they are
    // re-checked on the short, backed-off cadence, which is how the
    // worker's asynchronous result is picked up.
    let action = super::requeue_after(blueprint, &outcome);
    super::patch_reconcile_outcome_with(
        api,
        &ctx.recorder,
        &current,
        outcome,
        "blueprint applied",
        None,
        fields,
    )
    .await?;

    Ok(action)
}

/// Deleting the CR deletes the blueprint instance only. What it applied —
/// flows, stages, policies… — stays in Authentik, as when a blueprint file
/// is removed: to remove those, mark their entries `state: absent` and let
/// that apply before deleting the CR.
async fn cleanup(blueprint: &AuthentikBlueprint, ctx: &Ctx) -> Result<Action, Error> {
    if let Some(id) = blueprint
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;
        gateway.delete_blueprint(id).await.map_err(Error::Gateway)?;
        super::count_writes("AuthentikBlueprint", gateway.as_ref());
    }
    super::announce_deleted(&ctx.recorder, blueprint).await;
    Ok(Action::await_change())
}
