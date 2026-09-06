//! Layer-3 integration test, same shape as `group_controller.rs` (the
//! documented template every other CRD's integration test copies): real
//! `kube-apiserver` via `testkit::envtest`, the real `AuthentikScopeMapping`
//! controller, `AuthentikGateway` swapped for a `wiremock`-backed one.

use adapters_inbound::controller;
use api::AuthentikScopeMapping;
use api::scope_mapping::AuthentikScopeMappingSpec;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

const EXPRESSION: &str = "return {\"groups\": []}";

fn scope_mapping_response(pk: &str) -> serde_json::Value {
    serde_json::json!({
        "pk": pk,
        "managed": null,
        "name": "rustfs policies",
        "expression": EXPRESSION,
        "component": "ak-property-mapping-provider-scope-form",
        "verbose_name": "Scope Mapping",
        "verbose_name_plural": "Scope Mappings",
        "meta_model_name": "authentik_providers_oauth2.scopemapping",
        "scope_name": "groups",
        "description": null,
    })
}

fn scope_mapping(cr_name: &str) -> AuthentikScopeMapping {
    AuthentikScopeMapping {
        metadata: ObjectMeta {
            name: Some(cr_name.to_string()),
            ..Default::default()
        },
        spec: AuthentikScopeMappingSpec {
            name: "rustfs policies".to_string(),
            scope_name: "groups".to_string(),
            expression: EXPRESSION.to_string(),
            description: None,
        },
        status: None,
    }
}

#[tokio::test]
async fn scope_mapping_controller_syncs_authentik_id_onto_status() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let expected_pk = "88888888-8888-8888-8888-888888888888";
    mock.mock_post(
        "/propertymappings/provider/scope/",
        201,
        scope_mapping_response(expected_pk),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::scope_mapping::run(client.clone(), ctx));

    let mappings: Api<AuthentikScopeMapping> = Api::all(client.clone());
    mappings
        .create(&PostParams::default(), &scope_mapping("rustfs-policies"))
        .await
        .expect("AuthentikScopeMapping CR create must succeed");

    let result = wait_for(&mappings, "rustfs-policies", |mapping| {
        mapping.status.as_ref()?.authentik_id.clone()
    })
    .await;

    assert_eq!(result, expected_pk);
}

/// The `spec.name` collision path. Authentik answers a duplicate scope
/// mapping name with a 400 whose body carries "already exists" rather than
/// a 409, which is exactly the case `shared::is_duplicate_conflict` exists
/// for — so this asserts the reason code the *controller* writes, not just
/// the gateway mapping the contract test covers.
#[tokio::test]
async fn scope_mapping_controller_marks_errored_on_authentik_conflict() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_post(
        "/propertymappings/provider/scope/",
        400,
        serde_json::json!({"name": ["Scope Mapping with this name already exists."]}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::scope_mapping::run(client.clone(), ctx));

    let mappings: Api<AuthentikScopeMapping> = Api::all(client.clone());
    mappings
        .create(&PostParams::default(), &scope_mapping("rustfs-conflict"))
        .await
        .expect("AuthentikScopeMapping CR create must succeed");

    let (status, reason) = wait_for(&mappings, "rustfs-conflict", |mapping| {
        let s = mapping.status.as_ref()?;
        let cond = s.conditions.iter().find(|c| c.type_ == "Ready")?;
        Some((cond.status.clone(), cond.reason.clone()))
    })
    .await;

    assert_eq!(status, "False");
    assert_eq!(reason, "AuthentikObjectAlreadyExists");
}
