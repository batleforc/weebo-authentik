//! Layer-3: the admission webhook refuses an `AuthentikApplication` whose
//! explicit Vault path its secret store would refuse — outside
//! `<pathPrefix>/<namespace>/` and not opened by the instance's
//! `pathGrants` — against a real envtest cluster, whose CRD schema also
//! refuses an invalid grant.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failed step fails the test, with its message"
)]

use std::sync::Arc;

use adapters_inbound::webhook::{self, InstanceLookup, WebhookState};
use adapters_outbound::{AuthentikInstances, AuthentikSecretStoreFactory, KubePolicySource};
use api::AuthentikInstance;
use axum::body::Body;
use axum::http::Request;
use kube::api::{Api, PostParams};
use tower::ServiceExt;

fn instance(path_grants: serde_json::Value) -> AuthentikInstance {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "authentik.weebo.io/v1alpha1", "kind": "AuthentikInstance",
        "metadata": {"name": "main"},
        "spec": {
            "url": "https://login.example.com",
            "tokenSecretRef": {"name": "token", "namespace": "authentik", "key": "token"},
            "secretStore": {"vault": {
                "address": "https://vault:8200", "mount": "mv",
                "kubernetesAuthRole": "auth", "pathGrants": path_grants,
            }},
        },
    }))
    .unwrap()
}

fn review(namespace: &str, path: &str) -> Request<Body> {
    let object = serde_json::json!({
        "apiVersion": "authentik.weebo.io/v1alpha1", "kind": "AuthentikApplication",
        "metadata": {"name": "app", "namespace": namespace},
        "spec": {
            "instanceRef": {"name": "main"}, "name": "App", "slug": "app",
            "provider": {"kind": "saml"},
            "secretTargets": [{"backend": "vault", "name": "app", "path": path}],
        },
    });
    let body = serde_json::json!({
        "apiVersion": "admission.k8s.io/v1", "kind": "AdmissionReview",
        "request": {
            "uid": "11111111-1111-1111-1111-111111111111",
            "namespace": namespace,
            "kind": {"group": "authentik.weebo.io", "version": "v1alpha1", "kind": "AuthentikApplication"},
            "resource": {"group": "authentik.weebo.io", "version": "v1alpha1", "resource": "authentikapplications"},
            "operation": "CREATE", "userInfo": {}, "object": object,
        }
    });
    Request::builder()
        .method("POST")
        .uri("/validate")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn decision(client: &kube::Client, namespace: &str, path: &str) -> serde_json::Value {
    let instances = Arc::new(AuthentikInstances::new(client.clone()));
    let router = webhook::router(
        WebhookState {
            policies: Arc::new(KubePolicySource::new(client.clone())),
            instances: Some(InstanceLookup {
                client: client.clone(),
                directory: instances.clone(),
                vaults: Arc::new(AuthentikSecretStoreFactory::new(client.clone(), instances)),
            }),
        },
        Arc::new(weebo_kit_runtime::Health::default()),
        weebo_kit_runtime::WatchScope::cluster(),
    );
    let response = router.oneshot(review(namespace, path)).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn vault_paths_outside_the_namespace_need_a_grant() {
    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let api: Api<AuthentikInstance> = Api::all(client.clone());

    // The schema refuses a grant covering the whole mount.
    let whole_mount = instance(serde_json::json!([{"namespaces": ["auth"], "paths": ["+/*"]}]));
    let err = api
        .create(&PostParams::default(), &whole_mount)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("literal segment"), "{err}");

    api.create(
        &PostParams::default(),
        &instance(serde_json::json!([{"namespaces": ["auth"], "paths": ["batlehub/auth"]}])),
    )
    .await
    .unwrap();

    let granted = decision(&client, "auth", "batlehub/auth").await;
    assert_eq!(granted["response"]["allowed"], true, "{granted}");

    let own = decision(&client, "team-a", "weebo-authentik/team-a/app").await;
    assert_eq!(own["response"]["allowed"], true, "{own}");

    let refused = decision(&client, "team-a", "batlehub/auth").await;
    assert_eq!(refused["response"]["allowed"], false, "{refused}");
    assert_eq!(
        refused["response"]["status"]["reason"],
        domain::error::ReasonCode::SecretStoreError.as_str()
    );
    let message = refused["response"]["status"]["message"].as_str().unwrap();
    assert!(message.contains("namespace \"team-a\""), "{message}");
}
