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
    AuthentikGateway, BoundInstance, ClusterError, GatewayFactory, InstanceDirectory, PolicySource,
    RemoteWrite, SecretStoreFactory, Written,
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
use weebo_kit_runtime::shared::{SharedWatches, WatchesIn};
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
    /// The binary's shared watches ([`spawn_watches`]): one per kind for
    /// every controller, every policy re-check and every store read
    /// (`SharedWatches::default()`: each controller watches on its own).
    pub watches: SharedWatches,
}

/// One watch per kind, on `scope`, for every controller, the instance
/// stores and the policy reads; the cluster-scoped kinds only when the
/// scope watches them. `AuthentikNamespacePolicy` is watched both without
/// the label selector (`scope.without_selector()`: the policy checks read
/// every policy, whichever release selects it, so a change to any of them
/// re-checks the CRs) and through it (its controller, which reconciles
/// only the selected ones) — one watch when there is no selector. Spawned
/// before the lease is campaigned for: a standby keeps them warm.
pub fn spawn_watches(client: &Client, scope: &WatchScope, health: &Arc<Health>) -> SharedWatches {
    let mut w = SharedWatches::default();
    let (c, s, h) = (client, scope, health);
    if scope.cluster_kinds() {
        w.spawn(
            s.api::<api::AuthentikInstance>(c.clone()),
            s,
            h,
            "AuthentikInstance watch",
        );
        w.spawn(
            s.api::<api::AuthentikGroup>(c.clone()),
            s,
            h,
            "AuthentikGroup watch",
        );
        w.spawn(
            s.api::<api::AuthentikUser>(c.clone()),
            s,
            h,
            "AuthentikUser watch",
        );
        w.spawn(
            s.api::<api::AuthentikFlow>(c.clone()),
            s,
            h,
            "AuthentikFlow watch",
        );
        w.spawn(
            s.api::<api::AuthentikOutpost>(c.clone()),
            s,
            h,
            "AuthentikOutpost watch",
        );
        w.spawn(
            s.api::<api::AuthentikScopeMapping>(c.clone()),
            s,
            h,
            "AuthentikScopeMapping watch",
        );
        w.spawn(
            s.api::<api::AuthentikBrand>(c.clone()),
            s,
            h,
            "AuthentikBrand watch",
        );
        let all = s.without_selector();
        w.spawn(
            all.api::<api::AuthentikNamespacePolicy>(c.clone()),
            &all,
            h,
            "AuthentikNamespacePolicy watch",
        );
        w.spawn(
            s.api::<api::AuthentikNamespacePolicy>(c.clone()),
            s,
            h,
            "AuthentikNamespacePolicy selected watch",
        );
    }
    w.spawn(
        s.api::<api::AuthentikLocalInstance>(c.clone()),
        s,
        h,
        "AuthentikLocalInstance watch",
    );
    w.spawn(
        s.api::<api::AuthentikApplication>(c.clone()),
        s,
        h,
        "AuthentikApplication watch",
    );
    w.spawn(
        s.api::<api::AuthentikAccessPolicy>(c.clone()),
        s,
        h,
        "AuthentikAccessPolicy watch",
    );
    w
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
    /// A finalizer cleanup that couldn't complete, with its own reason
    /// code (its instance unresolvable, still in use…): the CR stays until
    /// it can, retried by `error_policy`.
    #[error(
        "cleanup failed: {}; it is retried — annotate with {}/force-orphan=\"true\" to release the CR without cleanup",
        .0.message,
        api::GROUP
    )]
    Cleanup(Failure),
}

impl Error {
    /// A cleanup stopped by `outcome`'s failure, keeping its reason code.
    pub fn cleanup(outcome: ReconcileOutcome) -> Self {
        match outcome {
            ReconcileOutcome::Errored { reason, message } => {
                Error::Cleanup(Failure::new(reason, message))
            }
            ReconcileOutcome::Synced { .. } => Error::Cleanup(Failure::new(
                ReasonCode::KubernetesApiError,
                "cleanup stopped without a failure",
            )),
        }
    }
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
            Error::Cleanup(failure) => (failure.reason, events::CLEANUP),
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
/// change rarely, and the check is a local list. On the policies' shared
/// watch (every policy, whatever its labels) when there is one, else its
/// own watch without the label selector. Without cluster-scoped kinds
/// (full namespaced) there is no policy to watch (`watches_in` skips the
/// cluster-scoped kind): every CR is bound to a local instance, which the
/// policy doesn't govern.
pub fn on_policy_change<K>(
    controller: kube::runtime::Controller<K>,
    client: &Client,
    ctx: &Ctx,
) -> kube::runtime::Controller<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    let governed = controller.store();
    controller.watches_in(
        &ctx.watches,
        &ctx.scope.without_selector(),
        client,
        move |_: api::AuthentikNamespacePolicy| {
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
/// re-check (drift correction), and so does a failure only a change can
/// clear ([`needs_a_change`]). Any other `Errored` outcome, though, is
/// often a transient cross-CRD ordering case — e.g. an
/// `AuthentikAccessPolicy` whose `applicationRef` exists but hasn't synced
/// its `authentikId` yet — and no controller here watches the CRD it
/// depends on, so nothing else wakes it. Requeuing those on the same
/// 5-minute cadence stalls convergence for up to 5 minutes (and times out
/// the e2e golden-path waits, which poll for ~150s); a short backoff,
/// matching `error_policy`'s hard-error cadence, lets the dependency
/// resolve in seconds — doubled per consecutive failure of `obj` (the
/// kit's per-CR backoff). Both jittered, so CRs created together don't
/// stay in lockstep.
pub fn requeue_after<K: Resource>(obj: &K, outcome: &ReconcileOutcome) -> Action {
    let steady = match outcome {
        ReconcileOutcome::Synced { .. } => true,
        ReconcileOutcome::Errored { reason, .. } => needs_a_change(*reason),
    };
    weebo_kit_runtime::controller::requeue_after(obj, steady, None)
}

/// Failures only a change of the CR (spec, annotation), of a namespace
/// policy, or of the remote object can clear: each CR or policy change
/// triggers a reconcile on its own, so retrying every ~30s only spends
/// Authentik API calls. The steady-state re-check still picks up a remote
/// change (the conflicting object deleted, the default brand released).
pub fn needs_a_change(reason: ReasonCode) -> bool {
    matches!(
        reason,
        ReasonCode::UnsupportedProviderKind
            | ReasonCode::InvalidProviderSpec
            | ReasonCode::DisruptiveProviderKindChangeNotAllowed
            | ReasonCode::AuthentikObjectAlreadyExists
            | ReasonCode::DefaultBrandConflict
            | ReasonCode::NamespaceNotAllowed
            | ReasonCode::InstanceNotAllowed
            | ReasonCode::InstanceBindingChanged
    )
}

/// The CR as the API server has it now, rather than the watch cache's
/// copy: reconciles for one object are serialized, but the watch event
/// carrying a *previous* reconcile's status patch can still be in flight
/// when this one starts (e.g. right behind the finalizer-add patch), so a
/// cached `authentikId: None` would be stale and trigger a spurious second
/// create — which the gateway can never recover from (see `ports.rs`,
/// "never a silent adopt"). Confirmed in practice on groups. Its status
/// also gives the freshest previous conditions for the status patch.
pub async fn current<K>(api: &Api<K>, obj: &K) -> Result<K, Error>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned,
{
    let name = obj.meta().name.clone().unwrap_or_default();
    Ok(api.get(&name).await?)
}

/// Every CR bound to `instance`, as `Kind/[namespace/]name` — what its
/// finalizer waits for: their own cleanups need the instance. An
/// `AuthentikApplication` is bound by its recorded `status.instance`, or
/// — not resolved yet — by its `instanceRef`; an `AuthentikAccessPolicy`
/// by its recorded `status.instance`. `default` (the instance is the only
/// `AuthentikInstance`): the cluster-scoped kinds, which live on the
/// default instance, count too once they own a remote object.
pub async fn instance_dependents(
    ctx: &Ctx,
    instance: &BoundInstance,
    default: bool,
) -> Result<Vec<String>, Error> {
    let mut all = Vec::new();
    let applications = ctx
        .scope
        .api::<api::AuthentikApplication>(ctx.client.clone())
        .list(&Default::default())
        .await?;
    all.extend(
        applications
            .items
            .iter()
            .filter(|a| {
                application_bound_to(
                    instance,
                    a.meta().namespace.as_deref(),
                    &a.spec.instance_ref,
                    a.authentik_status().and_then(|s| s.instance.as_ref()),
                )
            })
            .map(describe),
    );
    let policies = ctx
        .scope
        .api::<api::AuthentikAccessPolicy>(ctx.client.clone())
        .list(&Default::default())
        .await?;
    all.extend(
        policies
            .items
            .iter()
            .filter(|p| p.authentik_status().and_then(|s| s.instance.as_ref()) == Some(instance))
            .map(describe),
    );
    if default && ctx.scope.cluster_kinds() {
        all.extend(owning::<api::AuthentikGroup>(ctx).await?);
        all.extend(owning::<api::AuthentikUser>(ctx).await?);
        all.extend(owning::<api::AuthentikFlow>(ctx).await?);
        all.extend(owning::<api::AuthentikScopeMapping>(ctx).await?);
        all.extend(owning::<api::AuthentikOutpost>(ctx).await?);
        all.extend(owning::<api::AuthentikBrand>(ctx).await?);
    }
    Ok(all)
}

/// The CRs of the cluster-scoped `K` that own a remote object
/// (`status.authentikId`).
async fn owning<K>(ctx: &Ctx) -> Result<Vec<String>, Error>
where
    K: Resource<DynamicType = ()> + HasStatus + Clone + Debug + DeserializeOwned,
{
    let list = Api::<K>::all(ctx.client.clone())
        .list(&Default::default())
        .await?;
    Ok(list
        .items
        .iter()
        .filter(|o| {
            o.authentik_status()
                .is_some_and(|s| s.authentik_id.is_some())
        })
        .map(describe)
        .collect())
}

fn describe<K: Resource<DynamicType = ()>>(obj: &K) -> String {
    let meta = obj.meta();
    let name = meta.name.as_deref().unwrap_or_default();
    match &meta.namespace {
        Some(ns) => format!("{}/{ns}/{name}", K::kind(&())),
        None => format!("{}/{name}", K::kind(&())),
    }
}

/// Whether an application is bound to `instance`: recorded in its status,
/// or — not resolved yet — its `instanceRef` may designate it (by name
/// only, both kinds are candidates).
fn application_bound_to(
    instance: &BoundInstance,
    namespace: Option<&str>,
    instance_ref: &api::application::InstanceRef,
    recorded: Option<&BoundInstance>,
) -> bool {
    use api::application::InstanceKind;
    if let Some(recorded) = recorded {
        return recorded == instance;
    }
    let r = instance_ref;
    if r.name != instance.name || r.kind.is_some_and(|k| k != instance.kind) {
        return false;
    }
    match instance.kind {
        InstanceKind::Instance => r.namespace.is_none(),
        InstanceKind::LocalInstance => {
            r.namespace.as_deref().or(namespace) == instance.namespace.as_deref()
        }
    }
}

/// A finalizer's cleanup on an instance: released once no CR is bound to
/// it any more ([`instance_dependents`]), refused with `InstanceInUse`
/// before.
pub async fn release_instance(
    ctx: &Ctx,
    instance: &BoundInstance,
    default: bool,
) -> Result<Action, Error> {
    let dependents = instance_dependents(ctx, instance, default).await?;
    if dependents.is_empty() {
        return Ok(Action::await_change());
    }
    let shown: Vec<_> = dependents.iter().take(5).cloned().collect();
    Err(Error::Cleanup(Failure::new(
        ReasonCode::InstanceInUse,
        format!(
            "{} CR(s) still use {instance} (e.g. {}): delete them first",
            dependents.len(),
            shown.join(", ")
        ),
    )))
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
    patch_reconcile_outcome_with(
        api,
        recorder,
        obj,
        outcome,
        synced_message,
        instance,
        serde_json::Map::new(),
    )
    .await
}

/// [`patch_reconcile_outcome_on`], also merging the kind's own status
/// `fields` (e.g. an application's `providerId`).
pub async fn patch_reconcile_outcome_with<K>(
    api: &Api<K>,
    recorder: &Recorder,
    obj: &K,
    outcome: ReconcileOutcome,
    synced_message: &str,
    instance: Option<&BoundInstance>,
    mut fields: serde_json::Map<String, serde_json::Value>,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + HasStatus + Clone + Debug + DeserializeOwned + Serialize,
{
    let previous = obj
        .authentik_status()
        .map(|s| s.conditions.as_slice())
        .unwrap_or_default();
    let ready = ready_of(&outcome, synced_message);
    if let ReconcileOutcome::Synced {
        authentik_id: Some(id),
    } = &outcome
    {
        fields.insert("authentikId".into(), id.clone().into());
    }
    // Also recorded by a failed reconcile that created the object first.
    let written_id = fields
        .get("authentikId")
        .and_then(|id| id.as_str())
        .map(str::to_string);
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
    if let Some(id) = &written_id
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
    let reason = match outcome {
        ReconcileOutcome::Synced { .. } => ReasonCode::Reconciled,
        ReconcileOutcome::Errored { reason, .. } => *reason,
    };
    metrics().record(kind, reason, started.elapsed());
}

fn metrics() -> &'static ReconcileMetrics {
    static METRICS: OnceLock<ReconcileMetrics> = OnceLock::new();
    METRICS.get_or_init(|| ReconcileMetrics::new(FIELD_MANAGER, "weebo_authentik"))
}

/// Counts each write `gateway` made to Authentik
/// (`weebo_authentik_remote_writes_total{kind, cause}`); returns them.
/// Flat at rest: a steady `cause="drift"` rate is a field compared wrong
/// (weebo-si RFC 0003).
pub fn count_writes(kind: &'static str, gateway: &dyn AuthentikGateway) -> Vec<Written> {
    let writes = gateway.take_writes();
    for write in &writes {
        metrics().record_write(kind, write.cause);
    }
    writes
}

/// The note of a `DriftCorrected` Event: each object put back, with its
/// fields. `None` when nothing drifted.
pub fn drift_note(writes: &[Written]) -> Option<String> {
    let drifted: Vec<String> = writes
        .iter()
        .filter(|w| w.cause == RemoteWrite::Drift)
        .map(|w| format!("{} ({})", w.object, w.fields.join(", ")))
        .collect();
    (!drifted.is_empty()).then(|| format!("drift corrected in Authentik: {}", drifted.join("; ")))
}

/// After a reconcile: [`count_writes`], then a `Normal` `DriftCorrected`
/// Event naming what was put back — unless the spec just changed (`obj`
/// read fresh: its `observedGeneration` is the last applied one), which is
/// applied, not a drift.
pub async fn report_writes<K>(
    kind: &'static str,
    recorder: &Recorder,
    obj: &K,
    gateway: &dyn AuthentikGateway,
) where
    K: Resource<DynamicType = ()> + HasStatus,
{
    let writes = count_writes(kind, gateway);
    let spec_changed =
        obj.meta().generation != obj.authentik_status().and_then(|s| s.observed_generation);
    if spec_changed {
        return;
    }
    if let Some(note) = drift_note(&writes) {
        events::announce_change(
            recorder,
            &obj.object_ref(&()),
            ReasonCode::DriftCorrected,
            events::RECONCILE,
            &note,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::jiff::Timestamp;

    use super::*;

    fn at(secs: i64) -> Time {
        Time(Timestamp::from_second(secs).unwrap())
    }

    #[test]
    fn a_drift_note_names_each_object_and_its_fields() {
        let write = |object, cause, fields| Written {
            object,
            cause,
            fields,
        };
        assert_eq!(
            drift_note(&[write("application", RemoteWrite::Created, vec![])]),
            None
        );
        assert_eq!(
            drift_note(&[
                write("oauth2_provider", RemoteWrite::Drift, vec!["redirect_uris"]),
                write("policy_binding", RemoteWrite::Created, vec![]),
                write("application", RemoteWrite::Drift, vec!["name", "meta_icon"]),
            ])
            .as_deref(),
            Some(
                "drift corrected in Authentik: oauth2_provider (redirect_uris); \
                 application (name, meta_icon)"
            )
        );
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

    #[test]
    fn a_failure_only_a_change_clears_waits_for_the_steady_requeue() {
        assert!(needs_a_change(
            ReasonCode::DisruptiveProviderKindChangeNotAllowed
        ));
        assert!(needs_a_change(ReasonCode::AuthentikObjectAlreadyExists));
        assert!(needs_a_change(ReasonCode::DefaultBrandConflict));
        // A dependency not ready yet resolves on its own: retried soon.
        assert!(!needs_a_change(ReasonCode::ApplicationRefNotFound));
        assert!(!needs_a_change(ReasonCode::AuthentikApiError));
        assert!(!needs_a_change(ReasonCode::InstanceInUse));
    }

    #[test]
    fn a_cleanup_failure_keeps_its_reason() {
        let err = Error::cleanup(ReconcileOutcome::Errored {
            reason: ReasonCode::InstanceRefNotFound,
            message: "gone".into(),
        });
        assert_eq!(
            err.reason(),
            (ReasonCode::InstanceRefNotFound, events::CLEANUP)
        );
        assert!(err.to_string().contains("authentik.weebo.io/force-orphan"));
    }

    #[test]
    fn an_application_is_bound_by_its_status_then_by_its_ref() {
        use api::application::{InstanceKind, InstanceRef};
        let prod = BoundInstance::cluster("prod");
        let local = BoundInstance::local("team-a", "prod");
        let by_name = InstanceRef::named("prod");

        // Recorded: only that instance.
        assert!(application_bound_to(
            &local,
            Some("team-a"),
            &by_name,
            Some(&local)
        ));
        assert!(!application_bound_to(
            &prod,
            Some("team-a"),
            &by_name,
            Some(&local)
        ));
        // Not resolved yet, by name only: either kind may be it.
        assert!(application_bound_to(&prod, Some("team-a"), &by_name, None));
        assert!(application_bound_to(&local, Some("team-a"), &by_name, None));
        assert!(!application_bound_to(
            &local,
            Some("team-b"),
            &by_name,
            None
        ));
        // A pinned kind rules the other out.
        let pinned = InstanceRef {
            kind: Some(InstanceKind::Instance),
            ..by_name
        };
        assert!(!application_bound_to(&local, Some("team-a"), &pinned, None));
        assert!(!application_bound_to(
            &BoundInstance::cluster("other"),
            Some("team-a"),
            &pinned,
            None
        ));
    }
}
