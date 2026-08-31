//! One `kube::runtime::Controller` per CRD: real finalizer add/remove and
//! `status` patch plumbing, reconcile bodies delegate to
//! `application::use_cases::*` (fully implemented, not a stub — see
//! `.prompt/plan.md`, "Politique de mutation").

use std::fmt::Debug;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use application::ports::{GatewayFactory, SecretStoreFactory};
use application::use_cases::ReconcileOutcome;
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Client, Resource};
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{KeyValue, global};
use serde::Serialize;
use serde::de::DeserializeOwned;

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

/// Patches only the `Ready` condition via the `status` subresource. The
/// `ReasonCode`-only signature is the same type-level constraint as
/// `domain::error::ReasonCode` — a reconciler cannot compile a
/// hand-written reason string here.
pub async fn patch_ready_condition<K>(
    api: &Api<K>,
    name: &str,
    status: ConditionStatus,
    reason: ReasonCode,
    message: impl Into<String>,
) -> Result<(), kube::Error>
where
    K: Resource + Clone + Debug + DeserializeOwned + Serialize,
    K::DynamicType: Default,
{
    let status_str = match status {
        ConditionStatus::True => "True",
        ConditionStatus::False => "False",
        ConditionStatus::Unknown => "Unknown",
    };
    let patch = serde_json::json!({
        "status": {
            "conditions": [{
                "type": "Ready",
                "status": status_str,
                "reason": reason.as_str(),
                "message": message.into(),
            }]
        }
    });
    // `force` is Server-Side-Apply-only (`Patch::Apply`) — a real
    // apiserver rejects it paired with `Patch::Merge` ("PatchParams::force
    // only works with Patch::Apply"), caught by the layer-3 envtest
    // integration test, not by `cargo check`. `field_manager` alone is
    // fine (optional, but still attributes the change) for any patch type.
    api.patch_status(
        name,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Merge(&patch),
    )
    .await?;
    Ok(())
}

/// Shared `ReconcileOutcome` -> `status` patch translation — call this
/// instead of matching on `outcome` directly in each controller's
/// `apply()`. `synced_message` is used only for the `Ready` condition's
/// message on a successful sync (e.g. `"group synced"`). Correct even for
/// CRDs whose use-case never sets `authentik_id` (`AuthentikInstance`,
/// `AuthentikNamespacePolicy`): `Synced { authentik_id: Some(_) }` is
/// simply never constructed for those, so only the `None` arm below ever
/// fires for them.
pub async fn patch_reconcile_outcome<K>(
    api: &Api<K>,
    name: &str,
    outcome: ReconcileOutcome,
    synced_message: &str,
) -> Result<(), kube::Error>
where
    K: Resource + Clone + Debug + DeserializeOwned + Serialize,
    K::DynamicType: Default,
{
    match outcome {
        ReconcileOutcome::Synced {
            authentik_id: Some(id),
        } => patch_synced_status(api, name, &id, synced_message).await,
        ReconcileOutcome::Synced { authentik_id: None } => {
            patch_ready_condition(
                api,
                name,
                ConditionStatus::True,
                ReasonCode::Reconciled,
                synced_message,
            )
            .await
        }
        ReconcileOutcome::Errored { reason, message } => {
            patch_ready_condition(api, name, ConditionStatus::False, reason, message).await
        }
    }
}

/// OTLP-exported when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (see
/// `operator::telemetry::init`) — a safe no-op otherwise, since the
/// `opentelemetry::global` facade records nothing until a real
/// `MeterProvider` is installed, same as `tracing`'s macros being safe to
/// call before a subscriber is set.
const METER_NAME: &str = "weebo-authentik-operator";

static RECONCILE_TOTAL: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("weebo_authentik_reconcile_total")
        .with_description(
            "Reconcile attempts, by CRD kind, result (synced/errored), and reason code",
        )
        .build()
});

static RECONCILE_DURATION_SECONDS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .f64_histogram("weebo_authentik_reconcile_duration_seconds")
        .with_description("Reconcile duration in seconds, by CRD kind")
        .build()
});

/// Records one reconcile attempt's outcome — call from every controller's
/// `apply()` right after computing `outcome`, before patching status.
/// `kind` is the CRD's own kind string (e.g. `"AuthentikApplication"`),
/// `started` is when that `apply()` call began.
pub fn record_reconcile(kind: &'static str, started: Instant, outcome: &ReconcileOutcome) {
    let (result, reason) = match outcome {
        ReconcileOutcome::Synced { .. } => ("synced", ReasonCode::Reconciled.as_str()),
        ReconcileOutcome::Errored { reason, .. } => ("errored", reason.as_str()),
    };
    RECONCILE_TOTAL.add(
        1,
        &[
            KeyValue::new("kind", kind),
            KeyValue::new("result", result),
            KeyValue::new("reason", reason),
        ],
    );
    RECONCILE_DURATION_SECONDS.record(
        started.elapsed().as_secs_f64(),
        &[KeyValue::new("kind", kind)],
    );
}

/// Patches `status.authentikId` alongside `Ready: True` in a single
/// `status` subresource call. Without this, `status.authentikId` is never
/// persisted — the next reconcile would see `authentik_id: None` again and
/// attempt a second create against Authentik, which the "attempt-create-
/// first" contract (see `.prompt/plan.md`, "Modele de status commun")
/// turns into a spurious `AuthentikObjectAlreadyExists`. `authentik_id`
/// is `None` for CRDs with no remote object (`AuthentikInstance`,
/// `AuthentikNamespacePolicy`) — those never call this helper.
pub async fn patch_synced_status<K>(
    api: &Api<K>,
    name: &str,
    authentik_id: &str,
    message: impl Into<String>,
) -> Result<(), kube::Error>
where
    K: Resource + Clone + Debug + DeserializeOwned + Serialize,
    K::DynamicType: Default,
{
    let patch = serde_json::json!({
        "status": {
            "authentikId": authentik_id,
            "conditions": [{
                "type": "Ready",
                "status": "True",
                "reason": ReasonCode::Reconciled.as_str(),
                "message": message.into(),
            }]
        }
    });
    // `force` is Server-Side-Apply-only (`Patch::Apply`) — a real
    // apiserver rejects it paired with `Patch::Merge` ("PatchParams::force
    // only works with Patch::Apply"), caught by the layer-3 envtest
    // integration test, not by `cargo check`. `field_manager` alone is
    // fine (optional, but still attributes the change) for any patch type.
    api.patch_status(
        name,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Merge(&patch),
    )
    .await?;
    Ok(())
}
