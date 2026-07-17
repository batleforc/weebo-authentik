//! Layer-3 integration test: `AuthentikNamespacePolicy` has no
//! Authentik-side object (see
//! `application::use_cases::reconcile_namespace_policy`) — this just
//! proves the controller marks an accepted CR `Ready: True` without ever
//! calling the gateway (no mock mounted on the `AuthentikMock` server
//! below).

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use api::AuthentikNamespacePolicy;
use api::namespace_policy::AuthentikNamespacePolicySpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;
use testkit::static_gateway_factory::StaticGatewayFactory;

#[tokio::test]
async fn namespace_policy_controller_marks_ready() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let gateway = AuthentikHttpGateway::new(format!("{}/api/v3", mock.base_path()), "test-token");
    let gateway_factory = Arc::new(StaticGatewayFactory::new(Arc::new(gateway)));
    let secrets = Arc::new(K8sSecretStore::new(client.clone()));
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets,
    });

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

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let policy = policies
                .get("default-deny")
                .await
                .expect("CR must be gettable");
            if let Some(status) = &policy.status
                && status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("controller must mark AuthentikNamespacePolicy Ready within the timeout")
    });
}
