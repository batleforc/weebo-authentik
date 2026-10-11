//! Admission webhook: adapts an incoming `AdmissionReview` into a call to
//! `application::use_cases::evaluate_admission` (the kit's allow-list
//! engine, through `domain::allow_list`).

use std::sync::Arc;

use api::AuthentikNamespacePolicy;
use api::namespace_policy::{Effect as ApiEffect, ResourceKind as ApiResourceKind};
use application::use_cases::evaluate_admission::{
    AdmissionRequest as EvaluateAdmissionRequest, evaluate_admission,
};
use axum::Router;
use domain::allow_list::{self, Effect, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;
use kube::core::DynamicObject;
use kube::{Api, Client};
use weebo_kit_runtime::Health;
use weebo_kit_runtime::admission::{Admission, Denial};

#[derive(Clone)]
pub struct WebhookState {
    pub client: Client,
}

/// `POST /validate` (the kit's: a `DELETE`, or an `UPDATE` leaving `spec`
/// alone — the operator removing its finalizer — always passes), plus the
/// probes. Decisions are counted as `weebo_authentik_webhook_decisions_total`.
pub fn router(state: WebhookState, health: Arc<Health>) -> Router {
    weebo_kit_runtime::admission::router(
        Arc::new(state),
        health,
        "weebo-authentik-operator",
        "weebo_authentik",
    )
}

#[async_trait::async_trait]
impl Admission for WebhookState {
    type Reason = ReasonCode;

    async fn admit(
        &self,
        kind: &str,
        namespace: &str,
        _object: &DynamicObject,
    ) -> Result<(), Denial<ReasonCode>> {
        // The ValidatingWebhookConfiguration should only ever route
        // AuthentikApplication/AuthentikAccessPolicy here — an
        // unrecognized kind fails closed rather than assuming it's fine.
        let Some(kind) = parse_kind(kind) else {
            return Err(Denial::new(
                ReasonCode::NamespaceNotAllowed,
                format!("unrecognized kind for this webhook: {kind}"),
            ));
        };
        let (rules, policies_exist) = fetch_rules(&self.client).await.map_err(|err| {
            tracing::error!(error = %err, "failed to list AuthentikNamespacePolicy");
            // failurePolicy: Fail already blocks the request if this
            // webhook errors; this denial is the same fail-closed intent.
            Denial::new(
                ReasonCode::NamespaceNotAllowed,
                "AuthentikNamespacePolicy lookup failed",
            )
        })?;
        let result = evaluate_admission(
            &EvaluateAdmissionRequest { namespace, kind },
            &rules,
            policies_exist,
        );
        if result.allowed {
            Ok(())
        } else {
            Err(Denial::new(
                result.reason.unwrap_or(ReasonCode::NamespaceNotAllowed),
                result.message.unwrap_or_else(|| "denied".to_string()),
            ))
        }
    }

    fn undecidable(&self) -> ReasonCode {
        ReasonCode::NamespaceNotAllowed
    }
}

fn parse_kind(kind: &str) -> Option<ResourceKind> {
    match kind {
        "AuthentikApplication" => Some(ResourceKind::AuthentikApplication),
        "AuthentikAccessPolicy" => Some(ResourceKind::AuthentikAccessPolicy),
        _ => None,
    }
}

/// Returns the flattened rules plus whether any `AuthentikNamespacePolicy`
/// object exists at all — the two are distinct: a policy with an empty
/// `rules` list still counts as "exists" and keeps default-deny in force
/// (see `domain::allow_list::evaluate`).
async fn fetch_rules(client: &Client) -> Result<(Vec<NamespaceRule>, bool), kube::Error> {
    let api: Api<AuthentikNamespacePolicy> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    let policies_exist = !list.items.is_empty();
    let rules = list
        .into_iter()
        .flat_map(|policy| policy.spec.rules)
        .map(to_domain_rule)
        .collect();
    Ok((rules, policies_exist))
}

fn to_domain_rule(rule: api::namespace_policy::NamespaceRule) -> NamespaceRule {
    allow_list::rule(
        rule.namespaces,
        rule.allowed_kinds.into_iter().map(to_domain_kind).collect(),
        match rule.effect {
            ApiEffect::Allow => Effect::Allow,
            ApiEffect::Deny => Effect::Deny,
        },
    )
}

fn to_domain_kind(kind: ApiResourceKind) -> ResourceKind {
    match kind {
        ApiResourceKind::AuthentikApplication => ResourceKind::AuthentikApplication,
        ApiResourceKind::AuthentikAccessPolicy => ResourceKind::AuthentikAccessPolicy,
    }
}

#[cfg(test)]
mod tests {
    use kube::core::admission::AdmissionReview;

    use super::*;

    /// `validate`'s unrecognized-`kind` branch returns before ever touching
    /// `state.client` (see the `let Some(kind) = parse_kind(...) else`
    /// above), so this only needs a `Client` value that type-checks — no
    /// real cluster. `Client::try_from(Config::new(..))` builds the HTTP
    /// stack without making any network call.
    fn state_with_unreachable_client() -> WebhookState {
        let config = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
        WebhookState {
            client: Client::try_from(config)
                .expect("building a Client from a bare Config performs no network I/O"),
        }
    }

    /// Every field a real `kube-apiserver` always sends on an admission
    /// request — `uid`/`kind`/`resource`/`operation`/`userInfo` have no
    /// serde default, so a realistic fixture needs all of them, unlike the
    /// pre-typed-refactor test payload which only had `uid`/`namespace`/
    /// `kind.kind`.
    fn review_with_kind(kind: &str) -> AdmissionReview<DynamicObject> {
        serde_json::from_value(serde_json::json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "11111111-1111-1111-1111-111111111111",
                "namespace": "default",
                "kind": { "group": "authentik.weebo.io", "version": "v1alpha1", "kind": kind },
                "resource": {
                    "group": "authentik.weebo.io",
                    "version": "v1alpha1",
                    "resource": "authentikapplications",
                },
                "operation": "CREATE",
                "userInfo": {},
                "object": {
                    "apiVersion": "authentik.weebo.io/v1alpha1", "kind": kind,
                    "metadata": {"name": "x", "namespace": "default"}, "spec": {},
                },
            }
        }))
        .expect("fixture must deserialize into AdmissionReview<DynamicObject>")
    }

    #[tokio::test]
    async fn unrecognized_kind_fails_closed() {
        // Same "which rustls CryptoProvider" fix every real entry point in
        // this workspace applies before its first TLS-capable client is
        // built — see `testkit::envtest::EnvTestCluster::start`.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let review = review_with_kind("SomeUnrelatedKind");

        let response =
            weebo_kit_runtime::admission::handle(&state_with_unreachable_client(), review).await;

        let admission_response = response.response.expect("deny path always sets response");
        assert!(!admission_response.allowed);
        assert_eq!(
            admission_response.result.reason,
            ReasonCode::NamespaceNotAllowed.as_str()
        );
    }
}
