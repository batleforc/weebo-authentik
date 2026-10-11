use std::sync::Arc;

use api::AuthentikNamespacePolicy;
use application::use_cases::reconcile_namespace_policy::reconcile_namespace_policy;
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikNamespacePolicy> = Api::all(client);
    let health = ctx.health.clone();
    let results = super::controller(api)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikNamespacePolicy", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikNamespacePolicy>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikNamespacePolicy> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(policy) => apply(&api, &policy, &ctx).await,
            FinalizerEvent::Cleanup(_policy) => Ok(Action::await_change()),
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(
    api: &Api<AuthentikNamespacePolicy>,
    policy: &AuthentikNamespacePolicy,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let started = std::time::Instant::now();
    let outcome = reconcile_namespace_policy(policy);
    super::record_reconcile("AuthentikNamespacePolicy", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, policy, outcome, "policy accepted").await?;

    Ok(action)
}
