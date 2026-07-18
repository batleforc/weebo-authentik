//! Admission webhook — fully functional, not a stub. The underlying
//! `domain::allow_list` logic already exists; this just adapts an
//! incoming `AdmissionReview` into a call to
//! `application::use_cases::evaluate_admission`.

use std::sync::LazyLock;

use api::AuthentikNamespacePolicy;
use api::namespace_policy::{Effect as ApiEffect, ResourceKind as ApiResourceKind};
use application::use_cases::evaluate_admission::{AdmissionRequest, evaluate_admission};
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use domain::allow_list::{Effect, NamespaceRule, ResourceKind};
use domain::error::ReasonCode;
use kube::{Api, Client};
use opentelemetry::metrics::Counter;
use opentelemetry::{KeyValue, global};
use serde_json::{Value, json};

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

async fn validate(State(state): State<WebhookState>, Json(review): Json<Value>) -> Json<Value> {
    let uid = review["request"]["uid"].as_str().unwrap_or_default();
    let namespace = review["request"]["namespace"].as_str().unwrap_or_default();
    let kind_str = review["request"]["kind"]["kind"]
        .as_str()
        .unwrap_or_default();

    let Some(kind) = parse_kind(kind_str) else {
        // The ValidatingWebhookConfiguration should only ever route
        // AuthentikApplication/AuthentikAccessPolicy here — an
        // unrecognized kind fails closed rather than assuming it's fine.
        record_decision(kind_str, false);
        return Json(deny_response(
            uid,
            ReasonCode::NamespaceNotAllowed,
            &format!("unrecognized kind for this webhook: {kind_str}"),
        ));
    };

    let rules = match fetch_rules(&state.client).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::error!(error = %err, "failed to list AuthentikNamespacePolicy");
            // The ValidatingWebhookConfiguration's failurePolicy: Fail
            // already blocks the request cluster-wide if this webhook
            // 5xxs or times out; this explicit denial is a second,
            // in-band signal for the same fail-closed intent.
            record_decision(kind_str, false);
            return Json(deny_response(
                uid,
                ReasonCode::NamespaceNotAllowed,
                "AuthentikNamespacePolicy lookup failed",
            ));
        }
    };

    let result = evaluate_admission(&AdmissionRequest { namespace, kind }, &rules);
    record_decision(kind_str, result.allowed);

    if result.allowed {
        Json(allow_response(uid))
    } else {
        Json(deny_response(
            uid,
            result.reason.unwrap_or(ReasonCode::NamespaceNotAllowed),
            "namespace not allowed by any AuthentikNamespacePolicy",
        ))
    }
}

fn parse_kind(kind: &str) -> Option<ResourceKind> {
    match kind {
        "AuthentikApplication" => Some(ResourceKind::AuthentikApplication),
        "AuthentikAccessPolicy" => Some(ResourceKind::AuthentikAccessPolicy),
        _ => None,
    }
}

async fn fetch_rules(client: &Client) -> Result<Vec<NamespaceRule>, kube::Error> {
    let api: Api<AuthentikNamespacePolicy> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    Ok(list
        .into_iter()
        .flat_map(|policy| policy.spec.rules)
        .map(to_domain_rule)
        .collect())
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

fn allow_response(uid: &str) -> Value {
    json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview",
        "response": { "uid": uid, "allowed": true }
    })
}

fn deny_response(uid: &str, reason: ReasonCode, message: &str) -> Value {
    json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview",
        "response": {
            "uid": uid,
            "allowed": false,
            "status": { "code": 403, "reason": reason.as_str(), "message": message }
        }
    })
}
