//! Layer-3 integration test: `AuthentikInstance` has no Authentik-side
//! object (see `application::use_cases::reconcile_instance`) — this just
//! proves the controller marks an accepted CR `Ready: True` with no
//! `authentikId` and, crucially, without ever calling the gateway (no
//! mock is mounted on the `AuthentikMock` server below; an unexpected
//! call would 404 and this test would fail with a different reason).

use adapters_inbound::controller;
use api::AuthentikInstance;
use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

#[tokio::test]
async fn instance_controller_marks_ready_with_no_remote_object() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::instance::run(client.clone(), ctx));

    let instances: Api<AuthentikInstance> = Api::all(client.clone());
    instances
        .create(
            &PostParams::default(),
            &AuthentikInstance {
                metadata: ObjectMeta {
                    name: Some("prod".to_string()),
                    ..Default::default()
                },
                spec: AuthentikInstanceSpec {
                    url: "https://login.example.com".to_string(),
                    token_secret_ref: SecretKeyRef {
                        name: "authentik-token".to_string(),
                        namespace: "default".to_string(),
                        key: "token".to_string(),
                    },
                    tls: Default::default(),
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikInstance CR create must succeed");

    let authentik_id = wait_for(&instances, "prod", |instance| {
        instance
            .status
            .as_ref()
            .filter(|status| {
                status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .map(|status| status.authentik_id.clone())
    })
    .await;

    assert_eq!(authentik_id, None);
}
