use std::sync::Arc;

use api::AuthentikUser;
use application::use_cases::errored_from_factory_error;
use application::use_cases::reconcile_user::reconcile_user;
use kube::Client;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};

use super::{Ctx, Error, FINALIZER, error_policy};

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let api = ctx.scope.api::<AuthentikUser>(client.clone());
    let health = ctx.health.clone();
    let results = super::controller(api, &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive("AuthentikUser", health, results).await;
}

async fn reconcile(obj: Arc<AuthentikUser>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if super::import_pending(obj.as_ref()) {
        return Ok(Action::await_change());
    }
    let api: Api<AuthentikUser> = Api::all(ctx.client.clone());

    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(user) => apply(&api, &user, &ctx).await,
            FinalizerEvent::Cleanup(user) => {
                super::finalize(user.as_ref(), &ctx, cleanup(&api, &user, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(api: &Api<AuthentikUser>, user: &AuthentikUser, ctx: &Ctx) -> Result<Action, Error> {
    let current = super::current(api, user).await?;
    let user = &current;
    let authentik_id = user.status.as_ref().and_then(|s| s.authentik_id.clone());

    let started = std::time::Instant::now();
    let outcome = match ctx.gateway_factory.default_gateway().await {
        Ok(gateway) => reconcile_user(user, authentik_id.as_deref(), gateway.as_ref()).await,
        Err(e) => errored_from_factory_error(e),
    };
    super::record_reconcile("AuthentikUser", started, &outcome);
    let action = super::requeue_after(&outcome);
    super::patch_reconcile_outcome(api, &ctx.recorder, user, outcome, "user synced").await?;

    Ok(action)
}

async fn cleanup(
    _api: &Api<AuthentikUser>,
    user: &AuthentikUser,
    ctx: &Ctx,
) -> Result<Action, Error> {
    if let Some(id) = user.status.as_ref().and_then(|s| s.authentik_id.as_deref()) {
        let gateway = ctx
            .gateway_factory
            .default_gateway()
            .await
            .map_err(|e| Error::cleanup(errored_from_factory_error(e)))?;
        gateway
            .delete_user(id)
            .await
            .map_err(|e| Error::Gateway(e.to_string()))?;
    }
    super::announce_deleted(&ctx.recorder, user).await;
    Ok(Action::await_change())
}
