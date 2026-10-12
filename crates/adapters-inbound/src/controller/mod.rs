//! One `kube::runtime::Controller` per CRD: real finalizer add/remove and
//! `status` patch plumbing, reconcile bodies delegate to
//! `application::use_cases::*` (fully implemented, not a stub — see
//! `.prompt/plan.md`, "Politique de mutation").

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use api::AuthentikStatus;
use api::namespace_policy::NamespaceRule;
use api::status::Condition;
use application::ports::{
    BoundInstance, ClusterError, GatewayFactory, InstanceDirectory, PolicySource,
    SecretStoreFactory,
};
use application::use_cases::ReconcileOutcome;
use domain::Failure;
use domain::error::ReasonCode;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::events::Recorder;
use kube::{Client, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;
use weebo_kit_runtime::events;
use weebo_kit_runtime::metrics::ReconcileMetrics;
use weebo_kit_runtime::status::StatusUpdate;
use weebo_kit_runtime::{Health, WatchScope};

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
pub mod local_instance;
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
    /// Which instance an `AuthentikApplication` lives on (the kit's
    /// `Instances`, on the watch scope).
    pub instances: Arc<dyn InstanceDirectory>,
    /// Every `AuthentikNamespacePolicy`: the reconcilers of the kinds they
    /// govern re-check them, a policy may have changed since admission.
    pub policies: Arc<dyn PolicySource<NamespaceRule>>,
    /// Publishes an Event on every `Ready` transition.
    pub recorder: Recorder,
    pub shutdown: Shutdown,
    /// Liveness: controllers report their watch errors here.
    pub health: Arc<Health>,
    /// What the controllers watch (`WEEBO_WATCH_NAMESPACE`,
    /// `WEEBO_WATCH_LABEL_SELECTOR`, `WEEBO_WATCH_CLUSTER_KINDS`): every
    /// watch goes through it.
    pub scope: WatchScope,
}

/// The kit's controller skeleton: a watch filtered to what reconciling
/// acts on (spec, annotations, labels, finalizers, deletion — never the
/// CR's own status patches), bounded concurrency
/// (`WEEBO_RECONCILE_CONCURRENCY`), the result stream drained into the
/// liveness.
pub use weebo_kit_runtime::controller::{controller, drive};

/// Shared reconcile-error type for every controller. Not every variant is
/// constructed by every controller (e.g. `instance`/`namespace_policy` never
/// touch a gateway or secret store), which is fine for a `pub` type used
/// elsewhere in the crate — the alternative was 8 verbatim-copied enums.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube error: {0}")]
    Kube(#[from] kube::Error),
    /// Anything the finalizer helper reports: the reconcile's own error
    /// (`ApplyFailed`/`CleanupFailed`) or adding/removing the finalizer.
    #[error("{0}")]
    Finalizer(Box<kube::runtime::finalizer::Error<Error>>),
    #[error("authentik gateway error: {0}")]
    Gateway(String),
    #[error("secret store error: {0}")]
    SecretStore(String),
    /// Reading the cluster for the use-case (the namespace policies) failed.
    #[error(transparent)]
    Cluster(#[from] ClusterError),
}

/// Shared by every controller's `Controller::run(reconcile, error_policy,
/// ctx)` call — the body never depended on the CRD kind `K`, only on
/// `Error`/`Ctx`, so making it generic is a pure deletion of duplication.
impl Error {
    /// The reason code of the Event announcing this error, and whether it
    /// happened reconciling or cleaning up (the Event's `action`).
    pub fn reason(&self) -> (ReasonCode, &'static str) {
        use kube::runtime::finalizer::Error as Finalizer;
        match self {
            Error::Kube(_) | Error::Cluster(_) => {
                (ReasonCode::KubernetesApiError, events::RECONCILE)
            }
            Error::Gateway(_) => (ReasonCode::AuthentikApiError, events::RECONCILE),
            Error::SecretStore(_) => (ReasonCode::SecretStoreError, events::RECONCILE),
            Error::Finalizer(err) => match err.as_ref() {
                Finalizer::ApplyFailed(inner) => inner.reason(),
                Finalizer::CleanupFailed(inner) => (inner.reason().0, events::CLEANUP),
                _ => (ReasonCode::KubernetesApiError, events::RECONCILE),
            },
        }
    }
}

/// Every reconcile that errors out — before reaching a status patch, or in
/// a finalizer cleanup — is announced by a `Warning` Event on its CR (the
/// kit folds repeats into one series), then retried in ~30s (jittered).
pub fn error_policy<K>(obj: Arc<K>, err: &Error, ctx: Arc<Ctx>) -> Action
where
    K: Resource<DynamicType = ()>,
{
    let (reason, action) = err.reason();
    weebo_kit_runtime::controller::retry_after_error(
        &ctx.recorder,
        obj.object_ref(&()),
        reason,
        action,
        err,
    )
}

/// Re-checks every CR of `K` when an `AuthentikNamespacePolicy` changes:
/// a new policy may forbid them (or allow one denied so far). Policies
/// change rarely, and the check is a local list. Without cluster-scoped
/// kinds (full namespaced) there is no policy to watch: every CR is bound
/// to a local instance, which the policy doesn't govern.
pub fn on_policy_change<K>(
    controller: kube::runtime::Controller<K>,
    client: Client,
    scope: &WatchScope,
) -> kube::runtime::Controller<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    if !scope.cluster_kinds() {
        return controller;
    }
    let governed = controller.store();
    controller.watches(
        scope.api::<api::AuthentikNamespacePolicy>(client),
        scope.watcher_config(),
        move |_| {
            governed
                .state()
                .into_iter()
                .map(|o| kube::runtime::reflector::ObjectRef::from_obj(o.as_ref()))
                .collect::<Vec<_>>()
        },
    )
}

/// The namespace policy's refusal of a CR of `kind` in `namespace`, bound
/// to `instance` — `None` when it may go on. Only a CR bound to an
/// `AuthentikInstance` (or not bound yet) is governed: a local instance's
/// own token bounds what its CRs can do. Without cluster-scoped kinds
/// there is no policy at all.
pub async fn policy_refusal(
    ctx: &Ctx,
    namespace: &str,
    kind: domain::allow_list::ResourceKind,
    instance: Option<&BoundInstance>,
) -> Result<Option<ReconcileOutcome>, Error> {
    if !ctx.scope.cluster_kinds() || instance.is_some_and(|i| !i.governed_by_policy()) {
        return Ok(None);
    }
    let admission =
        application::use_cases::evaluate_admission::enforce(ctx.policies.as_ref(), namespace, kind)
            .await?;
    Ok(admission.refusal())
}

/// The instance an `AuthentikApplication` lives on: its `instanceRef`
/// resolved (the kit's rules), kept to what its status recorded.
pub async fn application_instance(
    ctx: &Ctx,
    app: &api::AuthentikApplication,
) -> Result<BoundInstance, application::ports::InstanceError> {
    use kube::ResourceExt;
    let recorded = app.authentik_status().and_then(|s| s.instance.as_ref());
    ctx.instances
        .resolve(
            app.namespace().as_deref(),
            Some(&app.spec.instance_ref),
            recorded,
        )
        .await
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
/// hard `Err`'s `error_policy` requeue, kept as one number. Both jittered
/// (the kit's), so CRs created together don't stay in lockstep.
pub fn requeue_after(outcome: &ReconcileOutcome) -> Action {
    weebo_kit_runtime::controller::requeue_after(
        matches!(outcome, ReconcileOutcome::Synced { .. }),
        None,
    )
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
    api::AuthentikLocalInstance,
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
    let ready = ready_of(outcome, synced_message);
    weebo_kit_runtime::conditions::conditions(previous, &ready, &[], generation, now)
}

/// `Ready`'s side of an outcome: `Ok(synced_message)` or the failure.
fn ready_of(outcome: &ReconcileOutcome, synced_message: &str) -> Result<String, Failure> {
    match outcome {
        ReconcileOutcome::Synced { .. } => Ok(synced_message.to_string()),
        ReconcileOutcome::Errored { reason, message } => Err(Failure::new(*reason, message)),
    }
}

/// A finalizer's cleanup — unless the CR carries
/// `authentik.weebo.io/force-orphan: "true"`, the way out for a CR whose
/// cleanup can't succeed any more (its instance or token gone) — then a
/// `Released` Event (the kit's).
pub async fn finalize<K>(
    obj: &K,
    ctx: &Ctx,
    cleanup: impl std::future::Future<Output = Result<Action, Error>>,
) -> Result<Action, Error>
where
    K: Resource<DynamicType = ()>,
{
    weebo_kit_runtime::controller::finalize(
        obj,
        &ctx.recorder,
        &api::ANNOTATIONS,
        ReasonCode::Released,
        async { cleanup.await.map(|_| ()) },
    )
    .await
}

/// Paused by the importer (`authentik.weebo.io/import-pending`): left
/// alone until its status is seeded. A deletion is always processed.
pub fn import_pending<K: Resource>(obj: &K) -> bool {
    api::ANNOTATIONS.import_pending(obj.meta())
}

/// Shared `ReconcileOutcome` -> `status` translation — call this instead of
/// matching on `outcome` in each controller's `apply()`. One merge patch on
/// the status subresource (conditions, `observedGeneration` on success,
/// `authentikId` when the outcome carries one), then Kubernetes Events: a
/// `Warning` for every failed reconcile, a `Normal` for every condition
/// change and for the remote object's creation. `obj` gives the previous
/// conditions, the generation and whether `authentikId` was already
/// recorded: pass the freshest copy at hand. `synced_message` is `Ready`'s
/// message on success (e.g. `"group synced"`).
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
    patch_reconcile_outcome_on(api, recorder, obj, outcome, synced_message, None).await
}

/// [`patch_reconcile_outcome`], recording `instance` — the one the CR was
/// resolved to — in `status.instance`. `None` leaves it as it was (a
/// refused resolution never erases the recorded binding).
pub async fn patch_reconcile_outcome_on<K>(
    api: &Api<K>,
    recorder: &Recorder,
    obj: &K,
    outcome: ReconcileOutcome,
    synced_message: &str,
    instance: Option<&BoundInstance>,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + HasStatus + Clone + Debug + DeserializeOwned + Serialize,
{
    let previous = obj
        .authentik_status()
        .map(|s| s.conditions.as_slice())
        .unwrap_or_default();
    let ready = ready_of(&outcome, synced_message);
    let mut fields = serde_json::Map::new();
    if let ReconcileOutcome::Synced {
        authentik_id: Some(id),
    } = &outcome
    {
        fields.insert("authentikId".into(), id.clone().into());
    }
    if let Some(instance) = instance {
        let instance = serde_json::to_value(instance).expect("a BoundInstance serializes");
        fields.insert("instance".into(), instance);
    }
    // The kit's: one merge patch (conditions, `observedGeneration` on
    // success), retried while the apiserver is unavailable, then a
    // `Warning` on every failed reconcile and a `Normal` for every
    // condition change. One more Event for the remote object's creation.
    weebo_kit_runtime::status::publish(
        api,
        recorder,
        FIELD_MANAGER,
        obj,
        previous,
        StatusUpdate {
            ready: &ready,
            advisories: &[],
            fields,
        },
    )
    .await?;
    let object = obj.object_ref(&());
    let recorded = obj
        .authentik_status()
        .and_then(|s| s.authentik_id.as_deref());
    if let ReconcileOutcome::Synced {
        authentik_id: Some(id),
    } = &outcome
        && recorded.is_none()
    {
        events::announce_change(
            recorder,
            &object,
            ReasonCode::Created,
            events::RECONCILE,
            &format!("created in Authentik as {id}"),
        )
        .await;
    }
    Ok(())
}

/// Call at the end of a successful finalizer cleanup: a `Normal` Event when
/// it deleted the CR's remote object.
pub async fn announce_deleted<K>(recorder: &Recorder, obj: &K)
where
    K: Resource<DynamicType = ()> + HasStatus,
{
    if let Some(id) = obj
        .authentik_status()
        .and_then(|s| s.authentik_id.as_deref())
    {
        events::announce_change(
            recorder,
            &obj.object_ref(&()),
            ReasonCode::Deleted,
            events::CLEANUP,
            &format!("deleted {id} from Authentik"),
        )
        .await;
    }
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
    fn errors_map_to_a_reason_and_an_action() {
        use kube::runtime::finalizer::Error as Finalizer;
        let gateway = || Error::Gateway("boom".into());
        assert_eq!(
            gateway().reason(),
            (ReasonCode::AuthentikApiError, events::RECONCILE)
        );
        assert_eq!(
            Error::Finalizer(Box::new(Finalizer::ApplyFailed(gateway()))).reason(),
            (ReasonCode::AuthentikApiError, events::RECONCILE)
        );
        assert_eq!(
            Error::Finalizer(Box::new(Finalizer::CleanupFailed(Error::SecretStore(
                "denied".into()
            ))))
            .reason(),
            (ReasonCode::SecretStoreError, events::CLEANUP)
        );
        assert_eq!(
            Error::Finalizer(Box::new(Finalizer::UnnamedObject)).reason(),
            (ReasonCode::KubernetesApiError, events::RECONCILE)
        );
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
