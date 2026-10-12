//! The status every CRD shares (`#[kube(status = "AuthentikStatus")]`).
//! See `.prompt/plan.md`, "Modele de status commun".

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use weebo_kit_api::{BoundInstance, Condition};

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
    /// The instance this object was resolved to and stays bound to
    /// (`AuthentikApplication`, and the `AuthentikAccessPolicy`s through
    /// their application): a `spec.instanceRef` by name only never
    /// silently moves to another instance created later under that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<BoundInstance>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
