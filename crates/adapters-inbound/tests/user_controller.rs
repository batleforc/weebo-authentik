//! Layer-3 integration test, same shape as `group_controller.rs` (the
//! documented template every other CRD's integration test copies): real
//! `kube-apiserver` via `testkit::envtest`, the real `AuthentikUser`
//! controller, `AuthentikGateway` swapped for a `wiremock`-backed one.

use adapters_inbound::controller;
use api::AuthentikUser;
use api::user::AuthentikUserSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

#[tokio::test]
async fn user_controller_syncs_authentik_id_onto_status() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let expected_pk = 3;
    mock.mock_post(
        "/core/users/",
        201,
        serde_json::json!({
            "pk": expected_pk, "username": "batleforc", "name": "batleforc",
            "is_active": true, "email": "batleforc@weebo.local",
            "date_joined": "2026-01-01T00:00:00Z", "is_superuser": false,
            "groups_obj": null, "roles_obj": null, "avatar": "",
            "uid": "batleforc-uid", "uuid": "44444444-4444-4444-4444-444444444444",
            "password_change_date": "2026-01-01T00:00:00Z", "last_updated": "2026-01-01T00:00:00Z",
        }),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::user::run(client.clone(), ctx));

    let users: Api<AuthentikUser> = Api::all(client.clone());
    users
        .create(
            &PostParams::default(),
            &AuthentikUser {
                metadata: ObjectMeta {
                    name: Some("batleforc".to_string()),
                    ..Default::default()
                },
                spec: AuthentikUserSpec {
                    username: "batleforc".to_string(),
                    name: "batleforc".to_string(),
                    email: "batleforc@weebo.local".to_string(),
                    is_active: true,
                    group_refs: vec![],
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikUser CR create must succeed");

    let result = wait_for(&users, "batleforc", |user| {
        user.status.as_ref()?.authentik_id.clone()
    })
    .await;

    assert_eq!(result, expected_pk.to_string());
}
