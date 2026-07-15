//! Kubernetes-side mirror of `domain::status`. Shared by every CRD's
//! `#[kube(status = "AuthentikStatus")]`. See `.prompt/plan.md`,
//! "Modele de status commun".

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub type_: String,
    /// `"True"`, `"False"`, or `"Unknown"`.
    pub status: String,
    /// One of `domain::error::ReasonCode`'s `as_str()` values — never a
    /// hand-written string.
    pub reason: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<Time>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
    /// The Authentik-side primary key. Presence means this CR actually
    /// owns/created the corresponding Authentik object — set once on
    /// first successful create (or seeded by the import tool), never
    /// inferred by name-matching. See `.prompt/plan.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentik_id: Option<String>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
