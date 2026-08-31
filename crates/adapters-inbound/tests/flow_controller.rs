//! Layer-3 integration test (`.prompt/plan.md`'s test strategy), the
//! `AuthentikFlow` analog of `group_controller.rs` (the documented
//! template): a real `kube-apiserver` + `etcd` via `testkit::envtest`, the
//! real `AuthentikFlow` controller running against it, and the real
//! `AuthentikHttpGateway` pointed at a `wiremock`-backed Authentik.
//!
//! Flows are slug-keyed, so `status.authentikId` ends up the flow's slug
//! (from the create response), not a numeric/UUID pk — mirroring
//! `application_controller.rs`.

use adapters_inbound::controller;
use api::AuthentikFlow;
use api::flow::{AuthentikFlowSpec, FlowDesignation};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for, wait_for_absence};

fn flow(slug: &str) -> AuthentikFlow {
    AuthentikFlow {
        metadata: ObjectMeta {
            name: Some(slug.to_string()),
            ..Default::default()
        },
        spec: AuthentikFlowSpec {
            slug: slug.to_string(),
            name: slug.to_string(),
            title: "Device code".to_string(),
            designation: FlowDesignation::Authentication,
            authentication: None,
            policy_engine_mode: None,
            compatibility_mode: None,
            layout: None,
            denied_action: None,
            background: None,
        },
        status: None,
    }
}

/// A minimal but schema-valid `Flow` create response — every field the
/// generated `authentik_client::models::Flow` requires to deserialize.
fn flow_response(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "pk": "33333333-3333-3333-3333-333333333333",
        "policybindingmodel_ptr_id": "44444444-4444-4444-4444-444444444444",
        "name": slug,
        "slug": slug,
        "title": "Device code",
        "designation": "authentication",
        "background_url": "/static/dist/assets/images/flow_background.jpg",
        "background_themed_urls": null,
        "stages": [],
        "policies": [],
        "cache_count": 0,
        "export_url": format!("/api/v3/flows/instances/{slug}/export/"),
    })
}

#[tokio::test]
async fn flow_controller_syncs_slug_onto_status() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post("/flows/instances/", 201, flow_response("device-code"))
        .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::flow::run(client.clone(), ctx));

    let flows: Api<AuthentikFlow> = Api::all(client.clone());
    flows
        .create(&PostParams::default(), &flow("device-code"))
        .await
        .expect("AuthentikFlow CR create must succeed");

    // The create response is slug-keyed, so `authentikId` is the slug.
    let synced = wait_for(&flows, "device-code", |f| {
        f.status.as_ref()?.authentik_id.clone()
    })
    .await;

    assert_eq!(synced, "device-code");
}

#[tokio::test]
async fn flow_controller_marks_errored_on_authentik_conflict() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post(
        "/flows/instances/",
        409,
        serde_json::json!({"slug": ["flow with this slug already exists."]}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::flow::run(client.clone(), ctx));

    let flows: Api<AuthentikFlow> = Api::all(client.clone());
    flows
        .create(&PostParams::default(), &flow("weebo-conflict"))
        .await
        .expect("AuthentikFlow CR create must succeed");

    let (status, reason) = wait_for(&flows, "weebo-conflict", |f| {
        let s = f.status.as_ref()?;
        let cond = s.conditions.iter().find(|c| c.type_ == "Ready")?;
        Some((cond.status.clone(), cond.reason.clone()))
    })
    .await;

    assert_eq!(status, "False");
    assert_eq!(reason, "AuthentikObjectAlreadyExists");
}

#[tokio::test]
async fn flow_controller_removes_finalizer_after_delete_calls_gateway() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post("/flows/instances/", 201, flow_response("weebo-deletable"))
        .await;
    // Flows are slug-keyed: delete hits `/flows/instances/{slug}/`.
    mock.mock_delete("/flows/instances/weebo-deletable/", 204)
        .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::flow::run(client.clone(), ctx));

    let flows: Api<AuthentikFlow> = Api::all(client.clone());
    flows
        .create(&PostParams::default(), &flow("weebo-deletable"))
        .await
        .expect("AuthentikFlow CR create must succeed");

    wait_for(&flows, "weebo-deletable", |f| {
        (f.status.as_ref().and_then(|s| s.authentik_id.as_deref()) == Some("weebo-deletable"))
            .then_some(())
    })
    .await;

    flows
        .delete("weebo-deletable", &Default::default())
        .await
        .expect("CR delete must succeed");

    wait_for_absence(&flows, "weebo-deletable").await;
}
