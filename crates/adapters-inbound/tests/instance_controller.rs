//! Layer-3 integration test: `AuthentikInstance` has no Authentik-side
//! object (see `application::use_cases::reconcile_instance`) — this just
//! proves the controller marks an accepted CR `Ready: True` with no
//! `authentikId` and, crucially, without ever calling the gateway (no
//! mock is mounted on the `AuthentikMock` server below; an unexpected
//! call would 404 and this test would fail with a different reason).

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use api::AuthentikInstance;
use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;
use testkit::static_gateway_factory::StaticGatewayFactory;

#[tokio::test]
async fn instance_controller_marks_ready_with_no_remote_object() {
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

    let ready = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let instance = instances.get("prod").await.expect("CR must be gettable");
            if let Some(status) = &instance.status
                && status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            {
                return instance;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("controller must mark AuthentikInstance Ready within the timeout"));

    assert_eq!(ready.status.as_ref().and_then(|s| s.authentik_id.as_deref()), None);
}
