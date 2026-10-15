use std::sync::Arc;

use api::AuthentikBrand;
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_brand::{
    BrandAction, BrandReconcileInput, BrandReconciled, plan_brand_action, reconcile_brand_recording,
};
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikBrand>(client.clone());
    let health = ctx.health.clone();
    let results = super::controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikBrand", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikBrand>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikBrand> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(brand) => apply(&api, &brand, &ctx).await,
            FinalizerEvent::Cleanup(brand) => {
                super::finalize(brand.as_ref(), &ctx, cleanup(&brand, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikBrand>,
    brand: &AuthentikBrand,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = brand.name_any();
    let started = std::time::Instant::now();
    let current = super::current(api, brand).await?;
    let brand = &current;
    let authentik_id = brand.status.as_ref().and_then(|s| s.authentik_id.clone());
    let this = to_input(&name, brand);

    // Every AuthentikBrand CR currently claiming spec.default: true.
    // Election is resolved Kubernetes-side, never against remote
    // Authentik state — see .prompt/plan.md, "Election du brand par
    // defaut".
    let all = list_default_claimants(api, &this).await?;
    let action = plan_brand_action(&this, &all);

    let BrandReconciled {
        outcome,
        authentik_id: written_id,
    } = match action {
        BrandAction::NoDefaultClaim => sync_brand(ctx, brand, authentik_id.as_deref(), false).await,
        BrandAction::BecomeDefault => sync_brand(ctx, brand, authentik_id.as_deref(), true).await,
        // Losing claimant: never touches Authentik at all, per
        // .prompt/plan.md, "Election du brand par defaut", point 3.
        BrandAction::Conflict {
            winner_name,
            reason,
        } => BrandReconciled {
            outcome: ReconcileOutcome::Errored {
                reason,
                message: format!(
                    "another AuthentikBrand ({winner_name}) already holds spec.default: true"
                ),
            },
            authentik_id: None,
        },
    };
    // The brand is recorded as soon as it exists in Authentik, even when
    // the default toggle failed afterwards: the next attempt patches it
    // instead of colliding with it, and the finalizer deletes it.
    let mut fields = serde_json::Map::new();
    if let Some(id) = written_id {
        fields.insert("authentikId".into(), id.into());
    }
    super::record_reconcile("AuthentikBrand", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome_with(
        api,
        &ctx.recorder,
        brand,
        outcome,
        "brand synced",
        None,
        fields,
    )
    .await?;

    Ok(action)
}

async fn sync_brand(
    ctx: &Ctx,
    brand: &AuthentikBrand,
    authentik_id: Option<&str>,
    become_default: bool,
) -> BrandReconciled {
    match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => {
            reconcile_brand_recording(brand, authentik_id, become_default, gateway.as_ref()).await
        }
        Err(e) => BrandReconciled {
            outcome: errored_from_factory_error(e),
            authentik_id: None,
        },
    }
}

async fn cleanup(brand: &AuthentikBrand, ctx: &Ctx) -> Result<Action, Error> {
    // Deliberately does NOT re-enable Authentik's native default brand if
    // no other CR claims default afterwards — see .prompt/plan.md,
    // "Election du brand par defaut", point 4. If another CR is still
    // claiming default, deleting this one changes its own status, which
    // the watcher already reconciles on — no extra plumbing needed here.
    if let Some(id) = brand
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
            .delete_brand(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, brand).await;
    Ok(Action::await_change())
}

fn to_input(name: &str, brand: &AuthentikBrand) -> BrandReconcileInput {
    BrandReconcileInput {
        name: name.to_string(),
        creation_timestamp: brand
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.as_second())
            .unwrap_or_default(),
        wants_default: brand.spec.default,
    }
}

async fn list_default_claimants(
    api: &Api<AuthentikBrand>,
    this: &BrandReconcileInput,
) -> Result<Vec<BrandReconcileInput>, Error> {
    let list = api.list(&Default::default()).await?;
    let mut claimants: Vec<BrandReconcileInput> = list
        .into_iter()
        .filter(|b| b.spec.default)
        .map(|b| to_input(&b.name_any(), &b))
        .collect();
    if this.wants_default && !claimants.iter().any(|c| c.name == this.name) {
        claimants.push(BrandReconcileInput {
            name: this.name.clone(),
            creation_timestamp: this.creation_timestamp,
            wants_default: this.wants_default,
        });
    }
    Ok(claimants)
}
