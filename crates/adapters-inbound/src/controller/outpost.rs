use std::sync::Arc;

use api::AuthentikOutpost;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_outpost::reconcile_outpost;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::runtime::watcher;
use kube::{Client, ResourceExt};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api: Api<AuthentikOutpost> = Api::all(client);
    Controller::new(api, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                tracing::error!(error = %err, "AuthentikOutpost reconcile failed");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<AuthentikOutpost>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let api: Api<AuthentikOutpost> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(outpost) => apply(&api, &outpost, &ctx).await,
            FinalizerEvent::Cleanup(outpost) => cleanup(&api, &outpost, &ctx).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

async fn apply(
    api: &Api<AuthentikOutpost>,
    outpost: &AuthentikOutpost,
    ctx: &Ctx,
) -> Result<Action, Error> {
    let name = outpost.name_any();
    let authentik_id = outpost.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_outpost(outpost, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikOutpost", started, &outcome);
    super::patch_reconcile_outcome(api, &name, outcome, "outpost synced").await?;

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

async fn cleanup(
    _api: &Api<AuthentikOutpost>,
    outpost: &AuthentikOutpost,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = outpost
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
            .delete_outpost(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    Ok(Action::await_change())
}
