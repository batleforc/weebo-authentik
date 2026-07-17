//! Layer-3 integration tests for `AuthentikBrand`'s default-election
//! logic (`.prompt/plan.md`, "Election du brand par defaut") against a
//! real `kube-apiserver` — `domain::brand_election` and
//! `application::use_cases::reconcile_brand` already have unit tests,
//! this proves the whole controller wiring (list-claimants, patch
//! status) behaves the same way for real.

use std::time::Duration;

use adapters_inbound::controller;
use api::AuthentikBrand;
use api::brand::AuthentikBrandSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

fn brand_spec(domain: &str, default: bool) -> AuthentikBrandSpec {
    AuthentikBrandSpec {
        domain: domain.to_string(),
        default,
        branding_title: None,
        branding_logo: None,
        branding_favicon: None,
        branding_default_flow_background: None,
        default_application_ref: None,
        flow_authentication: None,
        flow_invalidation: None,
        flow_recovery: None,
        flow_unenrollment: None,
        flow_user_settings: None,
    }
}

async fn wait_for_ready_status(api: &Api<AuthentikBrand>, name: &str) -> (String, Option<String>) {
    wait_for(api, name, |brand| {
        let status = brand.status.as_ref()?;
        let cond = status.conditions.iter().find(|c| c.type_ == "Ready")?;
        Some((cond.status.clone(), status.authentik_id.clone()))
    })
    .await
}

#[tokio::test]
async fn sole_default_brand_becomes_ready_and_default() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let expected_pk = "88888888-8888-8888-8888-888888888888";
    mock.mock_post(
        "/core/brands/",
        201,
        serde_json::json!({"brand_uuid": expected_pk, "domain": "weebo.example.com"}),
    )
    .await;
    mock.mock_get("/core/brands/", 200, empty_paginated_list())
        .await;
    mock.mock_patch(
        &format!("/core/brands/{expected_pk}/"),
        200,
        serde_json::json!({"brand_uuid": expected_pk, "domain": "weebo.example.com", "default": true}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::brand::run(client.clone(), ctx));

    let brands: Api<AuthentikBrand> = Api::all(client.clone());
    brands
        .create(
            &PostParams::default(),
            &AuthentikBrand {
                metadata: ObjectMeta {
                    name: Some("weebo".to_string()),
                    ..Default::default()
                },
                spec: brand_spec("weebo.example.com", true),
                status: None,
            },
        )
        .await
        .expect("AuthentikBrand CR create must succeed");

    let (status, authentik_id) = wait_for_ready_status(&brands, "weebo").await;
    assert_eq!(status, "True");
    assert_eq!(authentik_id.as_deref(), Some(expected_pk));
}

#[tokio::test]
async fn two_default_brands_conflict_loser_errored_without_touching_the_gateway() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let winner_pk = "99999999-9999-9999-9999-999999999999";
    // Only the winner's endpoints are mocked. If the loser's reconcile
    // regressed into calling the gateway anyway, wiremock would 404 the
    // unmocked request and the assertion on `reason` below would catch
    // it (AuthentikApiError instead of DefaultBrandConflict).
    mock.mock_post(
        "/core/brands/",
        201,
        serde_json::json!({"brand_uuid": winner_pk, "domain": "weebo-first.example.com"}),
    )
    .await;
    mock.mock_get("/core/brands/", 200, empty_paginated_list())
        .await;
    mock.mock_patch(
        &format!("/core/brands/{winner_pk}/"),
        200,
        serde_json::json!({"brand_uuid": winner_pk, "domain": "weebo-first.example.com", "default": true}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::brand::run(client.clone(), ctx));

    let brands: Api<AuthentikBrand> = Api::all(client.clone());
    brands
        .create(
            &PostParams::default(),
            &AuthentikBrand {
                metadata: ObjectMeta {
                    name: Some("weebo-first".to_string()),
                    ..Default::default()
                },
                spec: brand_spec("weebo-first.example.com", true),
                status: None,
            },
        )
        .await
        .expect("first AuthentikBrand CR create must succeed");

    // creationTimestamp has 1-second resolution server-side — a real gap
    // is needed so the second CR is unambiguously later than the first.
    tokio::time::sleep(Duration::from_secs(2)).await;

    brands
        .create(
            &PostParams::default(),
            &AuthentikBrand {
                metadata: ObjectMeta {
                    name: Some("weebo-second".to_string()),
                    ..Default::default()
                },
                spec: brand_spec("weebo-second.example.com", true),
                status: None,
            },
        )
        .await
        .expect("second AuthentikBrand CR create must succeed");

    let (winner_status, winner_id) = wait_for_ready_status(&brands, "weebo-first").await;
    assert_eq!(winner_status, "True");
    assert_eq!(winner_id.as_deref(), Some(winner_pk));

    let (loser_status, loser_id) = wait_for_ready_status(&brands, "weebo-second").await;
    assert_eq!(loser_status, "False");
    assert_eq!(loser_id, None);

    let loser = brands.get("weebo-second").await.unwrap();
    let reason = loser
        .status
        .as_ref()
        .unwrap()
        .conditions
        .iter()
        .find(|c| c.type_ == "Ready")
        .unwrap()
        .reason
        .clone();
    assert_eq!(reason, "DefaultBrandConflict");
}

fn empty_paginated_list() -> serde_json::Value {
    serde_json::json!({
        "pagination": {
            "next": 0, "previous": 0, "count": 0,
            "current": 1, "total_pages": 1, "start_index": 0, "end_index": 0
        },
        "results": [],
        "autocomplete": {}
    })
}
