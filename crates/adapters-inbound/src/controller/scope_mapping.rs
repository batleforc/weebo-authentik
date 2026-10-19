use std::sync::Arc;

use api::AuthentikScopeMapping;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_scope_mapping::reconcile_scope_mapping;
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikScopeMapping>(client.clone());
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikScopeMapping", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikScopeMapping>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikScopeMapping> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(mapping) => apply(&api, &mapping, &ctx).await,
            FinalizerEvent::Cleanup(mapping) => {
                super::finalize(mapping.as_ref(), &ctx, cleanup(&api, &mapping, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikScopeMapping>,
    mapping: &AuthentikScopeMapping,
    ctx: &Ctx,
) -> Result<Action, Error> {
    // Fresh from the API server, not the watch cache: see `super::current`.
    let current = super::current(api, mapping).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => {
            let outcome =
                reconcile_scope_mapping(mapping, authentik_id.as_deref(), gateway.as_ref()).await;
            super::report_writes(
                "AuthentikScopeMapping",
                &ctx.recorder,
                &current,
                gateway.as_ref(),
            )
            .await;
            outcome
        }
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikScopeMapping", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(
        api,
        &ctx.recorder,
        &current,
        outcome,
        "scope mapping synced",
    )
    .await?;

    Ok(action)
}

/// Deleting the CR deletes the Authentik mapping, even while a provider
/// still lists it in `propertyMappings`: Authentik drops it from that list
/// rather than refusing, and the provider then simply stops emitting the
/// claim. Nothing here checks for referencing providers first — the same
/// stance `AuthentikFlow`/`AuthentikGroup` take toward their own
/// slug/name references.
async fn cleanup(
    _api: &Api<AuthentikScopeMapping>,
    mapping: &AuthentikScopeMapping,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = mapping
        .status
        .as_ref()
        .and_then(|s| s.authentik_id.as_deref())
    {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;
        gateway
            .delete_scope_mapping(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
        super::count_writes("AuthentikScopeMapping", gateway.as_ref());
    }
    super::announce_deleted(&ctx.recorder, mapping).await;
    Ok(Action::await_change())
}
