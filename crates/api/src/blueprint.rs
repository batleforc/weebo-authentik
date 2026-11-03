use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// An Authentik **blueprint**, held inline: a `BlueprintInstance` whose
/// `content` is this CR's, applied by Authentik's own blueprint importer.
/// Cluster-scoped, like `AuthentikFlow`: what a blueprint declares (flows,
/// stages, policies, notification rules…) is instance-wide, never owned by
/// a namespace.
///
/// Blueprint instances are **pk-keyed** (a UUID), so `status.authentikId`
/// stores that pk.
///
/// # Why this kind exists
///
/// The other kinds model one Authentik object each, field by field. Most of
/// Authentik has no kind here — stages and their flow bindings, policies,
/// notification rules and transports, prompts… — and modelling them one by
/// one never ends: every stage type is its own model. A blueprint covers
/// them all in Authentik's own format, so this kind is the GitOps path for
/// whatever the typed kinds do not cover, without the alternative of
/// mounting files into Authentik's worker (no status, invisible to
/// Kubernetes).
///
/// # What `Ready` says
///
/// Authentik validates the content when the instance is written: an
/// invalid blueprint (an unknown model or field, a `!Find` that resolves to
/// nothing) is refused on the spot, `Ready: False` with `BlueprintInvalid`
/// and Authentik's validation log as the message. Retried with backoff, so
/// a blueprint that `!Find`s an object another CR has yet to create
/// converges on its own.
///
/// Applying is asynchronous (Authentik's worker): after a create or a
/// content change, the operator asks for an apply and reports
/// `BlueprintApplyPending` until the worker has applied **this** content,
/// then `Ready: True`. A failed apply is `BlueprintApplyFailed` and is
/// asked for again on each retry; the detail is in Authentik's task log
/// (System → Blueprints).
///
/// # What it does not do
///
/// - **Re-apply unchanged content.** An apply is only asked for when the
///   content stored in Authentik has not been applied yet, so an object
///   the blueprint created and someone edited in the UI keeps that edit
///   until the content changes. Every apply rewrites every entry (and
///   records an event per object), which is why it is not done on every
///   reconcile.
/// - **Delete what it created.** Deleting the CR deletes the blueprint
///   instance only; the flows, stages, policies… it applied stay, as they
///   do when a blueprint file is removed. To remove them, mark their
///   entries `state: absent`, let that apply, then delete the CR.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikBlueprint",
    plural = "authentikblueprints",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikBlueprintSpec {
    /// Authentik-side name of the blueprint instance (unique there). Not
    /// the CR's own name, nor the `metadata.name` inside `content`.
    pub name: String,
    /// The blueprint itself, in Authentik's format (`version: 1`,
    /// `metadata`, `entries`), YAML tags (`!KeyOf`, `!Find`, `!Context`…)
    /// included. Sent verbatim; Authentik validates it on every write.
    pub content: String,
    /// Values for the blueprint's `!Context` tags, merged over the
    /// `context` its own content declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "object", "x-kubernetes-preserve-unknown-fields" = true))]
    pub context: Option<serde_json::Value>,
    /// `false` keeps the instance but has Authentik skip it: nothing is
    /// applied, and what was applied stays as it is. Defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl AuthentikBlueprintSpec {
    /// `spec.enabled`, `true` when unset.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}
