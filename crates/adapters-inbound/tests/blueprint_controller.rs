//! Layer-3 integration test, same shape as `scope_mapping_controller.rs`:
//! real `kube-apiserver` via `testkit::envtest`, the real
//! `AuthentikBlueprint` controller, `AuthentikGateway` swapped for a
//! `wiremock`-backed one.

use adapters_inbound::controller;
use api::AuthentikBlueprint;
use api::blueprint::AuthentikBlueprintSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use sha2::{Digest, Sha512};
use testkit::authentik_mock::AuthentikMock;

mod support;
use support::{init_tracing, new_ctx, wait_for};

const PK: &str = "99999999-9999-9999-9999-999999999999";
/// What Authentik stores: the CR's content, trimmed.
const STORED: &str = "version: 1\nentries: []";

fn blueprint_response(status: &str, last_applied_hash: &str) -> serde_json::Value {
    serde_json::json!({
        "pk": PK,
        "name": "admin step-up",
        "path": "",
        "context": {},
        "last_applied": "2026-11-03T15:00:00Z",
        "last_applied_hash": last_applied_hash,
        "status": status,
        "enabled": true,
        "managed_models": [],
        "metadata": {},
        "content": STORED,
    })
}

fn blueprint(cr_name: &str) -> AuthentikBlueprint {
    AuthentikBlueprint {
        metadata: ObjectMeta {
            name: Some(cr_name.to_string()),
            ..Default::default()
        },
        spec: AuthentikBlueprintSpec {
            name: "admin step-up".to_string(),
            content: format!("{STORED}\n"),
            context: None,
            enabled: None,
        },
        status: None,
    }
}

fn ready(bp: &AuthentikBlueprint) -> Option<(String, String)> {
    let cond = bp
        .status
        .as_ref()?
        .conditions
        .iter()
        .find(|c| c.type_ == "Ready")?;
    Some((cond.status.clone(), cond.reason.clone()))
}

/// Created, its content already applied by the worker (the instance as
/// Authentik returns it carries the hash of that content): the pk is
/// recorded and `Ready` is `True` — with no apply asked for.
#[tokio::test]
async fn blueprint_controller_syncs_authentik_id_and_ready_once_applied() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let applied = format!("{:x}", Sha512::digest(STORED.as_bytes()));
    mock.mock_post(
        "/managed/blueprints/",
        201,
        blueprint_response("successful", &applied),
    )
    .await;
    mock.mock_get(
        &format!("/managed/blueprints/{PK}/"),
        200,
        blueprint_response("successful", &applied),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::blueprint::run(client.clone(), ctx));

    let blueprints: Api<AuthentikBlueprint> = Api::all(client.clone());
    blueprints
        .create(&PostParams::default(), &blueprint("admin-step-up"))
        .await
        .expect("AuthentikBlueprint CR create must succeed");

    let (id, ready) = wait_for(&blueprints, "admin-step-up", |bp| {
        let id = bp.status.as_ref()?.authentik_id.clone()?;
        let ready = ready(bp).filter(|(status, _)| status == "True")?;
        Some((id, ready))
    })
    .await;
    assert_eq!(id, PK);
    assert_eq!(ready.1, "Reconciled");

    let applies = mock
        .server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/apply/"))
        .count();
    assert_eq!(applies, 0, "applied content is not applied again");
}

/// Not applied yet: the apply is asked for, the pk is recorded all the
/// same, and `Ready` says it is pending.
#[tokio::test]
async fn blueprint_controller_records_the_id_while_the_apply_is_pending() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post(
        "/managed/blueprints/",
        201,
        blueprint_response("unknown", ""),
    )
    .await;
    mock.mock_get(
        &format!("/managed/blueprints/{PK}/"),
        200,
        blueprint_response("unknown", ""),
    )
    .await;
    mock.mock_post(
        &format!("/managed/blueprints/{PK}/apply/"),
        200,
        blueprint_response("unknown", ""),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::blueprint::run(client.clone(), ctx));

    let blueprints: Api<AuthentikBlueprint> = Api::all(client.clone());
    blueprints
        .create(&PostParams::default(), &blueprint("pending"))
        .await
        .expect("AuthentikBlueprint CR create must succeed");

    let (id, (status, reason)) = wait_for(&blueprints, "pending", |bp| {
        Some((bp.status.as_ref()?.authentik_id.clone()?, ready(bp)?))
    })
    .await;
    assert_eq!(id, PK);
    assert_eq!(status, "False");
    assert_eq!(reason, "BlueprintApplyPending");
}

/// Authentik's importer refuses the content: `BlueprintInvalid`, nothing
/// recorded.
#[tokio::test]
async fn blueprint_controller_marks_invalid_content() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post(
        "/managed/blueprints/",
        400,
        serde_json::json!({"content": ["Failed to validate blueprint", "- Entry invalid"]}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::blueprint::run(client.clone(), ctx));

    let blueprints: Api<AuthentikBlueprint> = Api::all(client.clone());
    blueprints
        .create(&PostParams::default(), &blueprint("invalid"))
        .await
        .expect("AuthentikBlueprint CR create must succeed");

    let (status, reason) = wait_for(&blueprints, "invalid", ready).await;
    assert_eq!(status, "False");
    assert_eq!(reason, "BlueprintInvalid");
    let recorded = blueprints
        .get("invalid")
        .await
        .expect("the CR is readable")
        .status
        .and_then(|s| s.authentik_id);
    assert_eq!(recorded, None);
}
