//! One `kube::runtime::Controller` per CRD: real finalizer add/remove and
//! `status` patch plumbing, reconcile bodies delegate to
//! `application::use_cases::*` (fully implemented, not a stub — see
//! `.prompt/plan.md`, "Politique de mutation").

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use api::AuthentikStatus;
use api::status::Condition;
use application::ports::{GatewayFactory, SecretStoreFactory};
use application::use_cases::ReconcileOutcome;
use domain::Failure;
use domain::error::ReasonCode;
use futures::{Stream, StreamExt};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::{self as runtime_controller, Action};
use kube::runtime::events::Recorder;
use kube::runtime::reflector::ObjectRef;
use kube::runtime::watcher;
use kube::{Client, Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use weebo_kit_runtime::Health;
use weebo_kit_runtime::metrics::ReconcileMetrics;

/// Resolves once the process is asked to stop: every controller then stops
/// taking new work, lets its in-flight reconciles finish, and its `run`
/// returns (`Controller::graceful_shutdown_on`).
pub use weebo_kit_runtime::Shutdown;

pub mod access_policy;
/// The `AuthentikApplication` controller. Named `app`, not `application`
/// — `controller::application` would shadow the `application` crate
/// (ports/use-cases) from every `use application::...` statement in this
/// module tree, since a local `mod` declaration wins over the extern
/// prelude for a bare leading path segment.
pub mod app;
pub mod brand;
pub mod flow;
pub mod group;
pub mod instance;
pub mod namespace_policy;
pub mod outpost;
pub mod scope_mapping;
pub mod user;

pub const FIELD_MANAGER: &str = "weebo-authentik-operator";
pub const FINALIZER: &str = "authentik.weebo.io/finalizer";

/// Shared across every controller. Concrete adapters are constructed and
/// injected once in `operator::main` — no controller here ever names
/// `AuthentikHttpGateway`/`K8sSecretStore`/`VaultSecretStore` directly.
pub struct Ctx {
    pub client: Client,
    pub gateway_factory: Arc<dyn GatewayFactory>,
    pub secrets_factory: Arc<dyn SecretStoreFactory>,
    /// Publishes an Event on every `Ready` transition.
    pub recorder: Recorder,
    pub shutdown: Shutdown,
    /// Liveness: controllers report their watch errors here.
    pub health: Arc<Health>,
}

/// Drains a controller's result stream until shutdown, reporting watch
/// failures to `health` — a watch erroring in a loop eventually fails
/// `/livez`. Reconcile failures are logged; they show in the CR's status.
pub async fn drive<K, E, S>(kind: &'static str, health: Arc<Health>, results: S)
where
    K: Resource,
    E: std::error::Error + Send + Sync + 'static,
    S: Stream<Item = Result<(ObjectRef<K>, Action), runtime_controller::Error<E, watcher::Error>>>,
{
    results
        .for_each(|res| {
            match res {
                Ok(_) => health.ok(kind),
                Err(runtime_controller::Error::QueueError(err)) => {
                    tracing::warn!(kind, error = %err, "watch error");
                    health.failed(kind, err.to_string());
                }
                Err(err) => tracing::error!(kind, error = %err, "reconcile failed"),
            }
            futures::future::ready(())
        })
        .await;
}

/// Shared reconcile-error type for every controller. Not every variant is
/// constructed by every controller (e.g. `instance`/`namespace_policy` never
/// touch a gateway or secret store), which is fine for a `pub` type used
/// elsewhere in the crate — the alternative was 8 verbatim-copied enums.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    #[error("finalizer error: {0}")]
    Finalizer(String),
    #[error("authentik gateway error: {0}")]
    Gateway(String),
    #[error("secret store error: {0}")]
    SecretStore(String),
}

/// Shared by every controller's `Controller::run(reconcile, error_policy,
/// ctx)` call — the body never depended on the CRD kind `K`, only on
/// `Error`/`Ctx`, so making it generic is a pure deletion of duplication.
pub fn error_policy<K>(_obj: Arc<K>, _err: &Error, _ctx: Arc<Ctx>) -> Action {
    Action::requeue(std::time::Duration::from_secs(30))
}

/// Requeue delay to return from a successful `apply()`, chosen from the
/// reconcile *outcome*. A clean `Synced` only needs the slow steady-state
/// re-check (drift correction). An `Errored` outcome, though, is often a
/// transient cross-CRD ordering case — e.g. an `AuthentikAccessPolicy`
/// whose `applicationRef` exists but hasn't synced its `authentikId` yet —
/// and no controller here watches the CRD it depends on, so nothing else
/// wakes it. Requeuing those on the same 5-minute cadence stalls
/// convergence for up to 5 minutes (and times out the e2e golden-path
/// waits, which poll for ~150s); a short backoff, matching `error_policy`'s
/// hard-error cadence, lets the dependency resolve in seconds. Mirrors a
/// hard `Err`'s `error_policy` requeue, kept as one number.
pub fn requeue_after(outcome: &ReconcileOutcome) -> Action {
    match outcome {
        ReconcileOutcome::Synced { .. } => Action::requeue(std::time::Duration::from_secs(300)),
        ReconcileOutcome::Errored { .. } => Action::requeue(std::time::Duration::from_secs(30)),
    }
}

/// The `status` of every CRD here is an `AuthentikStatus`; this gives the
/// shared helpers access to it without a trait per field.
pub trait HasStatus {
    fn authentik_status(&self) -> Option<&AuthentikStatus>;
}

macro_rules! impl_has_status {
    ($($kind:ty),+) => {$(
        impl HasStatus for $kind {
            fn authentik_status(&self) -> Option<&AuthentikStatus> {
                self.status.as_ref()
            }
        }
    )+};
}
impl_has_status!(
    api::AuthentikInstance,
    api::AuthentikGroup,
    api::AuthentikFlow,
    api::AuthentikUser,
    api::AuthentikOutpost,
    api::AuthentikScopeMapping,
    api::AuthentikBrand,
    api::AuthentikApplication,
    api::AuthentikAccessPolicy,
    api::AuthentikNamespacePolicy
);

/// The full condition list of `outcome` (the kit's: `Ready` first, then
/// one condition per advisory; `lastTransitionTime` only moves when a
/// condition flips). `synced_message` is `Ready`'s message on success.
pub fn conditions(
    previous: &[Condition],
    outcome: &ReconcileOutcome,
    synced_message: &str,
    generation: Option<i64>,
    now: &Time,
) -> Vec<Condition> {
    let ready = match outcome {
        ReconcileOutcome::Synced { .. } => Ok(synced_message.to_string()),
        ReconcileOutcome::Errored { reason, message } => Err(Failure::new(*reason, message)),
    };
    weebo_kit_runtime::conditions::conditions(previous, &ready, &[], generation, now)
}

/// Shared `ReconcileOutcome` -> `status` translation — call this instead of
/// matching on `outcome` in each controller's `apply()`. One merge patch on
/// the status subresource (conditions, `observedGeneration` on success,
/// `authentikId` when the outcome carries one), then a Kubernetes Event if
/// `Ready`'s status or reason changed. `obj` gives the previous conditions
/// and the generation; `synced_message` is `Ready`'s message on success
/// (e.g. `"group synced"`).
///
/// `authentikId` is written in the same call as `Ready: True`: without it,
/// the next reconcile would see `authentik_id: None` again and attempt a
/// second create against Authentik, which the "attempt-create-first"
/// contract (see `.prompt/plan.md`, "Modele de status commun") turns into a
/// spurious `AuthentikObjectAlreadyExists`.
pub async fn patch_reconcile_outcome<K>(
    api: &Api<K>,
    recorder: &Recorder,
    obj: &K,
    outcome: ReconcileOutcome,
    synced_message: &str,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + HasStatus + Clone + Debug + DeserializeOwned + Serialize,
{
    let previous = obj
        .authentik_status()
        .map(|s| s.conditions.as_slice())
        .unwrap_or_default();
    let generation = obj.meta().generation;
    let conds = conditions(
        previous,
        &outcome,
        synced_message,
        generation,
        &Time(Timestamp::now()),
    );
    let mut status = serde_json::json!({ "conditions": conds });
    if let ReconcileOutcome::Synced { authentik_id } = &outcome {
        status["observedGeneration"] = generation.into();
        if let Some(id) = authentik_id {
            status["authentikId"] = id.clone().into();
        }
    }
    // `force` is Server-Side-Apply-only (`Patch::Apply`) — a real
    // apiserver rejects it paired with `Patch::Merge`. `field_manager`
    // alone is fine for any patch type.
    api.patch_status(
        &obj.name_any(),
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Merge(serde_json::json!({ "status": status })),
    )
    .await?;

    weebo_kit_runtime::conditions::announce_ready_change(
        recorder,
        &obj.object_ref(&()),
        previous,
        &conds,
    )
    .await;
    Ok(())
}

/// Records one reconcile attempt's outcome as
/// `weebo_authentik_reconcile_total`/`_duration_seconds` (the kit's
/// `ReconcileMetrics`) — call from every controller's `apply()` right after
/// computing `outcome`. A no-op until `operator`'s telemetry installs a
/// real `MeterProvider`. `kind` is the CRD's kind, `started` when that
/// `apply()` call began.
pub fn record_reconcile(kind: &'static str, started: Instant, outcome: &ReconcileOutcome) {
    static METRICS: OnceLock<ReconcileMetrics> = OnceLock::new();
    let reason = match outcome {
        ReconcileOutcome::Synced { .. } => ReasonCode::Reconciled,
        ReconcileOutcome::Errored { reason, .. } => *reason,
    };
    METRICS
        .get_or_init(|| ReconcileMetrics::new(FIELD_MANAGER, "weebo_authentik"))
        .record(kind, reason, started.elapsed());
}

#[cfg(test)]
mod tests {
    use k8s_openapi::jiff::Timestamp;

    use super::*;

    fn at(secs: i64) -> Time {
        Time(Timestamp::from_second(secs).unwrap())
    }

    #[test]
    fn transition_time_only_moves_when_ready_flips() {
        let ok = ReconcileOutcome::Synced { authentik_id: None };
        let first = conditions(&[], &ok, "synced", Some(1), &at(10));
        assert_eq!(first[0].type_, "Ready");
        assert_eq!(first[0].reason, "Reconciled");
        assert_eq!(first[0].message, "synced");

        let again = conditions(&first, &ok, "synced", Some(2), &at(20));
        assert_eq!(again[0].last_transition_time, Some(at(10)));
        assert_eq!(again[0].observed_generation, Some(2));

        let failed = ReconcileOutcome::Errored {
            reason: ReasonCode::AuthentikApiError,
            message: "boom".into(),
        };
        let flipped = conditions(&again, &failed, "synced", Some(2), &at(30));
        assert_eq!(flipped[0].status, "False");
        assert_eq!(flipped[0].reason, "AuthentikApiError");
        assert_eq!(flipped[0].last_transition_time, Some(at(30)));
    }
}
