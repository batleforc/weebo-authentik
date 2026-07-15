use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::AuthentikStatus;

/// Namespace allow-list enforced by the admission webhook (and
/// double-checked by reconcilers). Cluster-scoped. Default-deny: a
/// `(namespace, kind)` pair is only allowed if some rule across every
/// `AuthentikNamespacePolicy` explicitly allows it — see
/// `domain::allow_list::evaluate`, which this CRD's rules map directly
/// onto.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "authentik.weebo.io",
    version = "v1alpha1",
    kind = "AuthentikNamespacePolicy",
    plural = "authentiknamespacepolicies",
    status = "AuthentikStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AuthentikNamespacePolicySpec {
    pub rules: Vec<NamespaceRule>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceRule {
    pub namespaces: Vec<String>,
    pub allowed_kinds: Vec<ResourceKind>,
    pub effect: Effect,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
pub enum ResourceKind {
    AuthentikApplication,
    AuthentikAccessPolicy,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum Effect {
    Allow,
    Deny,
}
