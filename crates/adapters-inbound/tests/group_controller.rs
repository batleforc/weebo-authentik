//! Layer-3 integration test (`.prompt/plan.md`'s test strategy): a real
//! `kube-apiserver` + `etcd` via `testkit::envtest`, the real
//! `AuthentikGroup` controller running against it, `AuthentikGateway`
//! swapped for a `wiremock`-backed one via `testkit::StaticGatewayFactory`.
//! This is the template every other CRD's integration test copies.

use adapters_inbound::controller;
use api::AuthentikGroup;
use api::group::AuthentikGroupSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for, wait_for_absence};

#[tokio::test]
async fn group_controller_syncs_authentik_id_onto_status() {
    init_tracing();

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

    let ctx = new_ctx(client.clone(), &mock);

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

    let synced = wait_for(&groups, "weebo-user", |group| {
        group.status.as_ref()?.authentik_id.clone()
    })
    .await;

    assert_eq!(synced, expected_pk);
}

#[tokio::test]
async fn group_controller_marks_errored_on_authentik_conflict() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_create_group(
        409,
        serde_json::json!({"name": ["group with this name already exists."]}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::group::run(client.clone(), ctx));

    let groups: Api<AuthentikGroup> = Api::all(client.clone());
    groups
        .create(
            &PostParams::default(),
            &AuthentikGroup {
                metadata: ObjectMeta {
                    name: Some("weebo-conflict".to_string()),
                    ..Default::default()
                },
                spec: AuthentikGroupSpec {
                    name: "weebo-conflict".to_string(),
                    is_superuser: false,
                    parent_ref: None,
                    attributes: Default::default(),
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikGroup CR create must succeed");

    let (status, reason) = wait_for(&groups, "weebo-conflict", |group| {
        let s = group.status.as_ref()?;
        let cond = s.conditions.iter().find(|c| c.type_ == "Ready")?;
        Some((cond.status.clone(), cond.reason.clone()))
    })
    .await;

    assert_eq!(status, "False");
    assert_eq!(reason, "AuthentikObjectAlreadyExists");
}

#[tokio::test]
async fn group_controller_removes_finalizer_after_delete_calls_gateway() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let pk = "22222222-2222-2222-2222-222222222222";
    mock.mock_create_group(
        201,
        serde_json::json!({
            "pk": pk, "num_pk": 1, "name": "weebo-deletable",
            "parents_obj": null, "users_obj": null, "roles_obj": [],
            "inherited_roles_obj": null, "children": [], "children_obj": null,
        }),
    )
    .await;
    mock.mock_delete(&format!("/core/groups/{pk}/"), 204).await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::group::run(client.clone(), ctx));

    let groups: Api<AuthentikGroup> = Api::all(client.clone());
    groups
        .create(
            &PostParams::default(),
            &AuthentikGroup {
                metadata: ObjectMeta {
                    name: Some("weebo-deletable".to_string()),
                    ..Default::default()
                },
                spec: AuthentikGroupSpec {
                    name: "weebo-deletable".to_string(),
                    is_superuser: false,
                    parent_ref: None,
                    attributes: Default::default(),
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikGroup CR create must succeed");

    wait_for(&groups, "weebo-deletable", |group| {
        (group
            .status
            .as_ref()
            .and_then(|s| s.authentik_id.as_deref())
            == Some(pk))
        .then_some(())
    })
    .await;

    groups
        .delete("weebo-deletable", &Default::default())
        .await
        .expect("CR delete must succeed");

    wait_for_absence(&groups, "weebo-deletable").await;
}
