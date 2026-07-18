//! One `kube::runtime::Controller` per CRD: real finalizer add/remove and
//! `status` patch plumbing, reconcile bodies delegate to
//! `application::use_cases::*` (mostly still `todo!()` there — see
//! `.prompt/plan.md`, "Politique de mutation").

use std::fmt::Debug;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use application::ports::{GatewayFactory, SecretStoreFactory};
use application::use_cases::ReconcileOutcome;
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use kube::api::{Api, Patch, PatchParams};
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
