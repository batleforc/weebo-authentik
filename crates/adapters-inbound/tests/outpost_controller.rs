//! Layer-3 integration test, same shape as `group_controller.rs` (the
//! documented template every other CRD's integration test copies): real
//! `kube-apiserver` via `testkit::envtest`, the real `AuthentikOutpost`
//! controller, `AuthentikGateway` swapped for a `wiremock`-backed one.

use adapters_inbound::controller;
use api::AuthentikOutpost;
use api::outpost::{AuthentikOutpostSpec, OutpostType};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

#[tokio::test]
async fn outpost_controller_syncs_authentik_id_onto_status() {
    init_tracing();

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

    let ctx = new_ctx(client.clone(), &mock);

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

    let result = wait_for(&outposts, "edge", |outpost| {
        outpost.status.as_ref()?.authentik_id.clone()
    })
    .await;

    assert_eq!(result, expected_pk);
}
