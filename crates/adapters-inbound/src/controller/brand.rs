use std::sync::Arc;

use api::AuthentikBrand;
use application::use_cases::ReconcileOutcome;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_brand::{
    BrandAction, BrandReconcileInput, plan_brand_action, reconcile_brand,
};
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
    let api: Api<AuthentikBrand> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikBrand reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikBrand>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikBrand> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(brand) => apply(&api, &brand, &ctx).await,
            FinalizerEvent::Cleanup(brand) => cleanup(&brand, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikBrand>,
    brand: &AuthentikBrand,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = brand.name_any();
    let started = std::time::Instant::now();
    let authentik_id = brand.status.as_ref().and_then(|s| s.authentik_id.clone());
    let this = to_input(&name, brand);

    // Every AuthentikBrand CR currently claiming spec.default: true.
    // Election is resolved Kubernetes-side, never against remote
    // Authentik state — see .prompt/plan.md, "Election du brand par
    // defaut".
    let all = list_default_claimants(api, &this).await?;
    let action = plan_brand_action(&this, &all);

    let outcome = match action {
        BrandAction::NoDefaultClaim => sync_brand(ctx, brand, authentik_id.as_deref(), false).await,
        BrandAction::BecomeDefault => sync_brand(ctx, brand, authentik_id.as_deref(), true).await,
        // Losing claimant: never touches Authentik at all, per
        // .prompt/plan.md, "Election du brand par defaut", point 3.
        BrandAction::Conflict {
            winner_name,
            reason,
        } => ReconcileOutcome::Errored {
            reason,
            message: format!(
                "another AuthentikBrand ({winner_name}) already holds spec.default: true"
            ),
        },
    };
    super::record_reconcile("AuthentikBrand", started, &outcome);

    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => {
            patch_synced_status(api, &name, &id, "brand synced").await?;
        }
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                &name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                "brand synced",
            )
            .await?;
        }
        ReconcileOutcome::Errored { reason, message } => {
            patch_ready_condition(api, &name, ConditionStatus::False, reason, message).await?;
        }
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

async fn sync_brand(
    ctx: &Ctx,
    brand: &AuthentikBrand,
    authentik_id: Option<&str>,
    become_default: bool,
) -> ReconcileOutcome {
    match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_brand(brand, authentik_id, become_default, gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
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
            .map_err(|e| Error::Gateway(e.to_string()))?;
        gateway
            .delete_brand(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
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

fn error_policy(_obj: Arc<AuthentikBrand>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}
