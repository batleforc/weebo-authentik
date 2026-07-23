//! Layer-3 integration test for the admission webhook's actual allow/deny
//! decision — `crates/operator/tests/webhook_tls.rs` only proves the TLS
//! transport works and that one default-deny case comes back; this drives
//! `webhook::router` directly (via `tower::ServiceExt::oneshot`, no HTTP/
//! TLS layer) against a real envtest cluster with real
//! `AuthentikNamespacePolicy` CRs, covering the decision logic itself:
//! an explicitly allowed namespace/kind, an explicitly denied one, and an
//! unrecognized `kind.kind` (the fail-closed branch that never resolves a
//! `ResourceKind` at all).
//!
//! Not covered here: `fetch_rules`'s own `kube::Error` branch (a genuine
//! Kubernetes API list failure). Reaching that against a real, healthy
//! envtest apiserver would need deliberately broken RBAC or a torn-down
//! client mid-test — more test-harness complexity than the branch itself
//! (a one-line `tracing::error!` + fixed deny) warrants.

use adapters_inbound::webhook::{self, WebhookState};
use api::AuthentikNamespacePolicy;
use api::namespace_policy::{AuthentikNamespacePolicySpec, Effect, NamespaceRule, ResourceKind};
use axum::body::Body;
use axum::http::Request;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::envtest::EnvTestCluster;
use tower::ServiceExt;

async fn create_policy(client: &kube::Client, name: &str, rules: Vec<NamespaceRule>) {
    let api: Api<AuthentikNamespacePolicy> = Api::all(client.clone());
    api.create(
        &PostParams::default(),
        &AuthentikNamespacePolicy {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: AuthentikNamespacePolicySpec { rules },
            status: None,
        },
    )
    .await
    .expect("AuthentikNamespacePolicy CR create must succeed");
}

fn review_request(namespace: &str, kind: &str) -> Request<Body> {
    let body = serde_json::json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview",
        "request": {
            "uid": "11111111-1111-1111-1111-111111111111",
            "namespace": namespace,
            "kind": { "group": "authentik.weebo.io", "version": "v1alpha1", "kind": kind },
            "resource": {
                "group": "authentik.weebo.io", "version": "v1alpha1",
                "resource": "authentikapplications",
            },
            "operation": "CREATE",
            "userInfo": {},
        }
    });
    Request::builder()
        .method("POST")
        .uri("/validate")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn decision(client: kube::Client, namespace: &str, kind: &str) -> serde_json::Value {
    let router = webhook::router(WebhookState { client });
    let response = router
        .oneshot(review_request(namespace, kind))
        .await
        .expect("router must handle the request");
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body must be readable");
    serde_json::from_slice(&body_bytes).expect("response body must be valid JSON")
}

#[tokio::test]
async fn explicitly_allowed_namespace_and_kind_is_allowed() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    create_policy(
        &client,
        "allow-team-a",
        vec![NamespaceRule {
            namespaces: vec!["team-a".to_string()],
            allowed_kinds: vec![ResourceKind::AuthentikApplication],
            effect: Effect::Allow,
        }],
    )
    .await;

    let review = decision(client, "team-a", "AuthentikApplication").await;
    assert_eq!(review["response"]["allowed"], true);
}

#[tokio::test]
async fn namespace_with_no_matching_rule_defaults_to_denied() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    create_policy(
        &client,
        "allow-team-a",
        vec![NamespaceRule {
            namespaces: vec!["team-a".to_string()],
            allowed_kinds: vec![ResourceKind::AuthentikApplication],
            effect: Effect::Allow,
        }],
    )
    .await;

    let review = decision(client, "team-b", "AuthentikApplication").await;
    assert_eq!(review["response"]["allowed"], false);
    assert_eq!(
        review["response"]["status"]["reason"],
        domain::error::ReasonCode::NamespaceNotAllowed.as_str()
    );
}

#[tokio::test]
async fn explicit_deny_rule_wins_over_an_allow_rule() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    create_policy(
        &client,
        "allow-then-deny",
        vec![
            NamespaceRule {
                namespaces: vec!["team-a".to_string()],
                allowed_kinds: vec![ResourceKind::AuthentikApplication],
                effect: Effect::Allow,
            },
            NamespaceRule {
                namespaces: vec!["team-a".to_string()],
                allowed_kinds: vec![ResourceKind::AuthentikApplication],
                effect: Effect::Deny,
            },
        ],
    )
    .await;

    let review = decision(client, "team-a", "AuthentikApplication").await;
    assert_eq!(review["response"]["allowed"], false);
}

#[tokio::test]
async fn no_policies_at_all_allows_every_namespace() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    // No AuthentikNamespacePolicy created — with no policy to enforce yet,
    // every namespace is allowed rather than default-denied.
    let review = decision(client, "team-a", "AuthentikApplication").await;
    assert_eq!(review["response"]["allowed"], true);
}

#[tokio::test]
async fn a_policy_that_covers_other_namespaces_still_denies_uncovered_ones() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    // A policy exists (covering team-a only) — this must NOT be treated
    // as "no policy", so team-b still default-denies.
    create_policy(
        &client,
        "allow-team-a",
        vec![NamespaceRule {
            namespaces: vec!["team-a".to_string()],
            allowed_kinds: vec![ResourceKind::AuthentikApplication],
            effect: Effect::Allow,
        }],
    )
    .await;

    let review = decision(client, "team-b", "AuthentikApplication").await;
    assert_eq!(review["response"]["allowed"], false);
}

#[tokio::test]
async fn unrecognized_kind_fails_closed_against_a_real_cluster() {
    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    // No AuthentikNamespacePolicy at all — proves this is the
    // unrecognized-kind branch specifically (unconditional deny before
    // `fetch_rules` is ever called), not just the default-deny-with-no-
    // rules branch covered by the other tests above.
    let review = decision(client, "team-a", "SomeUnrelatedKind").await;
    assert_eq!(review["response"]["allowed"], false);
    assert_eq!(
        review["response"]["status"]["reason"],
        domain::error::ReasonCode::NamespaceNotAllowed.as_str()
    );
}
