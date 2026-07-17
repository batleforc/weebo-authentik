//! Layer-3 integration test, same shape as `group_controller.rs` (the
//! documented template every other CRD's integration test copies): real
//! `kube-apiserver` via `testkit::envtest`, the real `AuthentikOutpost`
//! controller, `AuthentikGateway` swapped for a `wiremock`-backed one.

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use api::AuthentikOutpost;
use api::outpost::{AuthentikOutpostSpec, OutpostType};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;
use testkit::static_gateway_factory::StaticGatewayFactory;

#[tokio::test]
async fn outpost_controller_syncs_authentik_id_onto_status() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let expected_pk = "77777777-7777-7777-7777-777777777777";
    mock.mock_post(
        "/outposts/instances/",
        201,
        serde_json::json!({
            "pk": expected_pk, "name": "edge", "type": "proxy",
            "providers": [], "providers_obj": [], "service_connection_obj": null,
            "refresh_interval_s": 86400, "token_identifier": "tok", "config": {},
        }),
    )
    .await;

    let gateway = AuthentikHttpGateway::new(format!("{}/api/v3", mock.base_path()), "test-token");
    let gateway_factory = Arc::new(StaticGatewayFactory::new(Arc::new(gateway)));
    let secrets = Arc::new(K8sSecretStore::new(client.clone()));
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory,
        secrets,
    });

    tokio::spawn(controller::outpost::run(client.clone(), ctx));

    let outposts: Api<AuthentikOutpost> = Api::all(client.clone());
    outposts
        .create(
            &PostParams::default(),
            &AuthentikOutpost {
                metadata: ObjectMeta {
                    name: Some("edge".to_string()),
                    ..Default::default()
                },
                spec: AuthentikOutpostSpec {
                    name: "edge".to_string(),
                    r#type: OutpostType::Proxy,
                    config: serde_json::json!({}),
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikOutpost CR create must succeed");

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let outpost = outposts.get("edge").await.expect("CR must be gettable");
            if let Some(status) = &outpost.status
                && let Some(id) = &status.authentik_id
            {
                return id.clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("controller must sync status.authentikId within the timeout"));

    assert_eq!(result, expected_pk);
}
