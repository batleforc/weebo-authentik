//! One `kube::runtime::Controller` per CRD: real finalizer add/remove and
//! `status` patch plumbing, reconcile bodies delegate to
//! `application::use_cases::*` (mostly still `todo!()` there — see
//! `.prompt/plan.md`, "Politique de mutation").

use std::fmt::Debug;
use std::sync::Arc;

use application::ports::{GatewayFactory, SecretStore};
use domain::error::ReasonCode;
use domain::status::ConditionStatus;
use kube::api::{Api, Patch, PatchParams};
use kube::{Client, Resource};
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
    pub secrets: Arc<dyn SecretStore>,
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
