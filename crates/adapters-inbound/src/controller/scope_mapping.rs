use std::sync::Arc;

use api::AuthentikScopeMapping;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_scope_mapping::reconcile_scope_mapping;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikScopeMapping> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikScopeMapping reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikScopeMapping>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikScopeMapping> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(mapping) => apply(&api, &mapping, &ctx).await,
            FinalizerEvent::Cleanup(mapping) => cleanup(&api, &mapping, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikScopeMapping>,
    mapping: &AuthentikScopeMapping,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = mapping.name_any();
    // Re-fetched directly from the API server rather than trusted from
    // `mapping` (sourced from kube-runtime's local watch cache): reconciles
    // for one object are serialized, but the watch event carrying a
    // *previous* reconcile's `patch_synced_status` can still be in flight
    // when this one starts (e.g. right behind the finalizer-add patch),
    // so a cached `authentik_id: None` here would be stale and trigger a
    // spurious second `create_scope_mapping` — which the gateway can never
    // recover from (see `ports.rs`, "never a silent adopt"). Confirmed in
    // practice: without this, the finalizer-add-triggered reconcile that
    // immediately follows a successful create still reads a stale
    // `None` and re-creates, colliding with the object it just made.
    let current = api.get(&name).await?;
    let authentik_id = current.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => {
            reconcile_scope_mapping(mapping, authentik_id.as_deref(), gateway.as_ref()).await
        }
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikScopeMapping", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &name, outcome, "scope mapping synced").await?;

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
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_scope_mapping(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    Ok(Action::await_change())
}
