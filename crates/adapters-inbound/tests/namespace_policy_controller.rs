//! Layer-3 integration test: `AuthentikNamespacePolicy` has no
//! Authentik-side object (see
//! `application::use_cases::reconcile_namespace_policy`) — this just
//! proves the controller marks an accepted CR `Ready: True` without ever
//! calling the gateway (no mock mounted on the `AuthentikMock` server
//! below).

use adapters_inbound::controller;
use api::AuthentikNamespacePolicy;
use api::namespace_policy::AuthentikNamespacePolicySpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

#[tokio::test]
async fn namespace_policy_controller_marks_ready() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::namespace_policy::run(client.clone(), ctx));

    let policies: Api<AuthentikNamespacePolicy> = Api::all(client.clone());
    policies
        .create(
            &PostParams::default(),
            &AuthentikNamespacePolicy {
                metadata: ObjectMeta {
                    name: Some("default-deny".to_string()),
                    ..Default::default()
                },
                spec: AuthentikNamespacePolicySpec { rules: vec![] },
                status: None,
            },
        )
        .await
        .expect("AuthentikNamespacePolicy CR create must succeed");

    wait_for(&policies, "default-deny", |policy| {
        policy.status.as_ref().and_then(|status| {
            status
                .conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
                .then_some(())
        })
    })
    .await;
}
