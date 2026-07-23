//! Admission webhook — fully functional, not a stub. The underlying
//! `domain::allow_list` logic already exists; this just adapts an
//! incoming `AdmissionReview` into a call to
//! `application::use_cases::evaluate_admission`.

use std::sync::LazyLock;

use api::AuthentikNamespacePolicy;
use api::namespace_policy::{Effect as ApiEffect, ResourceKind as ApiResourceKind};
use application::use_cases::evaluate_admission::{
    AdmissionRequest as EvaluateAdmissionRequest, evaluate_admission,
};
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use domain::allow_list::{Effect, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;
use kube::core::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use kube::{Api, Client};
use opentelemetry::metrics::Counter;
use opentelemetry::{KeyValue, global};

/// OTLP-exported when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (see
/// `operator::telemetry::init`) — a safe no-op otherwise, same rationale
/// as `controller::RECONCILE_TOTAL`.
static WEBHOOK_DECISIONS_TOTAL: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("weebo-authentik-operator")
        .u64_counter("weebo_authentik_webhook_decisions_total")
        .with_description("Admission webhook decisions, by CRD kind and result (allow/deny)")
        .build()
});

fn record_decision(kind: &str, allowed: bool) {
    WEBHOOK_DECISIONS_TOTAL.add(
        1,
        &[
            KeyValue::new("kind", kind.to_string()),
            KeyValue::new("result", if allowed { "allow" } else { "deny" }),
        ],
    );
}

#[derive(Clone)]
pub struct WebhookState {
    pub client: Client,
}

pub fn router(state: WebhookState) -> Router {
    Router::new()
        .route("/validate", post(validate))
        .with_state(state)
}

async fn validate(
    State(state): State<WebhookState>,
    Json(review): Json<AdmissionReview<DynamicObject>>,
) -> Json<AdmissionReview<DynamicObject>> {
    let req: AdmissionRequest<DynamicObject> = match review.try_into() {
        Ok(req) => req,
        // No `request` to build a proper response from at all — mirrors
        // `AdmissionResponse::invalid`'s own documented use case.
        Err(_) => {
            record_decision("<missing request>", false);
            return Json(
                AdmissionResponse::invalid("AdmissionReview is missing its request").into_review(),
            );
        }
    };

    let kind_str = req.kind.kind.clone();
    let Some(kind) = parse_kind(&kind_str) else {
        // The ValidatingWebhookConfiguration should only ever route
        // AuthentikApplication/AuthentikAccessPolicy here — an
        // unrecognized kind fails closed rather than assuming it's fine.
        record_decision(&kind_str, false);
        return Json(deny(
            &req,
            ReasonCode::NamespaceNotAllowed,
            &format!("unrecognized kind for this webhook: {kind_str}"),
        ));
    };

    let namespace = req.namespace.clone().unwrap_or_default();

    let (rules, policies_exist) = match fetch_rules(&state.client).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::error!(error = %err, "failed to list AuthentikNamespacePolicy");
            // The ValidatingWebhookConfiguration's failurePolicy: Fail
            // already blocks the request cluster-wide if this webhook
            // 5xxs or times out; this explicit denial is a second,
            // in-band signal for the same fail-closed intent.
            record_decision(&kind_str, false);
            return Json(deny(
                &req,
                ReasonCode::NamespaceNotAllowed,
                "AuthentikNamespacePolicy lookup failed",
            ));
        }
    };

    let result = evaluate_admission(
        &EvaluateAdmissionRequest {
            namespace: &namespace,
            kind,
        },
        &rules,
        policies_exist,
    );
    record_decision(&kind_str, result.allowed);

    if result.allowed {
        Json(AdmissionResponse::from(&req).into_review())
    } else {
        Json(deny(
            &req,
            result.reason.unwrap_or(ReasonCode::NamespaceNotAllowed),
            "namespace not allowed by any AuthentikNamespacePolicy",
        ))
    }
}

/// `AdmissionResponse::deny` only sets `.result.message` — `code`/`reason`
/// are set separately here to keep the same `status.code: 403` +
/// `status.reason: <ReasonCode>` shape this webhook has always returned.
fn deny(
    req: &AdmissionRequest<DynamicObject>,
    reason: ReasonCode,
    message: &str,
) -> AdmissionReview<DynamicObject> {
    let mut response = AdmissionResponse::from(req).deny(message);
    response.result.code = 403;
    response.result.reason = reason.as_str().to_string();
    response.into_review()
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
    NamespaceRule {
        namespaces: rule.namespaces,
        allowed_kinds: rule.allowed_kinds.into_iter().map(to_domain_kind).collect(),
        effect: match rule.effect {
            ApiEffect::Allow => Effect::Allow,
            ApiEffect::Deny => Effect::Deny,
        },
    }
}

fn to_domain_kind(kind: ApiResourceKind) -> ResourceKind {
    match kind {
        ApiResourceKind::AuthentikApplication => ResourceKind::AuthentikApplication,
        ApiResourceKind::AuthentikAccessPolicy => ResourceKind::AuthentikAccessPolicy,
    }
}

#[cfg(test)]
mod tests {
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

        let Json(response) = validate(State(state_with_unreachable_client()), Json(review)).await;

        let admission_response = response.response.expect("deny path always sets response");
        assert!(!admission_response.allowed);
        assert_eq!(
            admission_response.result.reason,
            ReasonCode::NamespaceNotAllowed.as_str()
        );
    }
}
