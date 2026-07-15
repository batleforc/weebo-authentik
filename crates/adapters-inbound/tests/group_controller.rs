//! Layer-3 integration test (`.prompt/plan.md`'s test strategy): a real
//! `kube-apiserver` + `etcd` via `testkit::envtest`, the real
//! `AuthentikGroup` controller running against it, `AuthentikGateway`
//! swapped for a `wiremock`-backed one via `testkit::StaticGatewayFactory`.
//! This is the template every other CRD's integration test copies.

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use api::AuthentikGroup;
use api::group::AuthentikGroupSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;
use testkit::static_gateway_factory::StaticGatewayFactory;

#[tokio::test]
async fn group_controller_syncs_authentik_id_onto_status() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let expected_pk = "11111111-1111-1111-1111-111111111111";
    mock.mock_create_group(
        201,
        serde_json::json!({
            "pk": expected_pk,
            "num_pk": 1,
            "name": "weebo-user",
            "parents_obj": null,
            "users_obj": null,
            "roles_obj": [],
            "inherited_roles_obj": null,
            "children": [],
            "children_obj": null,
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

    tokio::spawn(controller::group::run(client.clone(), ctx));

    let groups: Api<AuthentikGroup> = Api::all(client.clone());
    groups
        .create(
            &PostParams::default(),
            &AuthentikGroup {
                metadata: ObjectMeta {
                    name: Some("weebo-user".to_string()),
                    ..Default::default()
                },
                spec: AuthentikGroupSpec {
                    name: "weebo-user".to_string(),
                    is_superuser: false,
                    parent_ref: None,
                    attributes: Default::default(),
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikGroup CR create must succeed");

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let group = groups
                .get("weebo-user")
                .await
                .expect("AuthentikGroup CR must be gettable");
            if let Some(status) = &group.status
                && let Some(id) = &status.authentik_id
            {
                return id.clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;

    let synced = match result {
        Ok(id) => id,
        Err(_) => {
            let group = groups.get("weebo-user").await.unwrap();
            panic!(
                "controller must sync status.authentikId within the timeout; last status: {:?}",
                group.status
            );
        }
    };

    assert_eq!(synced, expected_pk);
}
