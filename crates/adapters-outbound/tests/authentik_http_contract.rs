//! Layer-2 contract tests (`.prompt/plan.md`'s test strategy): script
//! Authentik API responses via `wiremock` (through
//! `testkit::authentik_mock::AuthentikMock`) and assert `AuthentikHttpGateway`
//! maps them correctly — in particular the 400/409/404 -> `GatewayError`
//! mapping in `authentik_http.rs`'s `map_err`, which every use-case's
//! "Modele de status commun" (collision rejection, idempotent delete)
//! depends on. No real Authentik instance involved, unlike layer 4.

use adapters_outbound::AuthentikHttpGateway;
use api::application::{Oauth2ProviderSpec, ProviderKind, RedirectUri};
use api::brand::AuthentikBrandSpec;
use api::flow::{
    AuthentikFlowSpec, FlowAuthentication, FlowDeniedAction, FlowDesignation, FlowLayout,
    PolicyEngineMode,
};
use api::group::AuthentikGroupSpec;
use api::outpost::{AuthentikOutpostSpec, OutpostType};
use api::{AuthentikBrand, AuthentikFlow, AuthentikGroup, AuthentikOutpost};
use application::ports::{AuthentikGateway, GatewayError};
use kube::api::ObjectMeta;
use testkit::authentik_mock::AuthentikMock;

fn gateway(mock: &AuthentikMock) -> AuthentikHttpGateway {
    AuthentikHttpGateway::new(format!("{}/api/v3", mock.base_path()), "test-token")
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

fn group(name: &str) -> AuthentikGroup {
    AuthentikGroup {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        spec: AuthentikGroupSpec {
            name: name.to_string(),
            is_superuser: false,
            parent_ref: None,
            attributes: Default::default(),
        },
        status: None,
    }
}

#[tokio::test]
async fn create_group_success_returns_the_authentik_pk() {
    let mock = AuthentikMock::start().await;
    let expected_pk = "11111111-1111-1111-1111-111111111111";
    mock.mock_create_group(
        201,
        serde_json::json!({
            "pk": expected_pk, "num_pk": 1, "name": "weebo-user",
            "parents_obj": null, "users_obj": null, "roles_obj": [],
            "inherited_roles_obj": null, "children": [], "children_obj": null,
        }),
    )
    .await;

    let result = gateway(&mock).create_group(&group("weebo-user")).await;
    assert_eq!(result.unwrap(), expected_pk);
}

#[tokio::test]
async fn create_group_conflict_maps_to_already_exists() {
    let mock = AuthentikMock::start().await;
    mock.mock_create_group(409, serde_json::json!({"name": ["already exists"]}))
        .await;

    let err = gateway(&mock)
        .create_group(&group("weebo-user"))
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::AlreadyExists(_)));
}

/// This exact body shape (`{"name": ["Group with this name already
/// exists."]}`, HTTP 400) is what a real Authentik instance returns for a
/// name collision — confirmed live against the sidecar Authentik in this
/// workspace, not guessed.
#[tokio::test]
async fn create_group_400_with_already_exists_body_maps_to_already_exists() {
    let mock = AuthentikMock::start().await;
    mock.mock_create_group(
        400,
        serde_json::json!({"name": ["Group with this name already exists."]}),
    )
    .await;

    let err = gateway(&mock)
        .create_group(&group("weebo-user"))
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::AlreadyExists(_)));
}

/// A genuine validation error also arrives as HTTP 400 (confirmed live:
/// a missing/unresolvable oauth2 provider flow reference returns this
/// same status with a body that never says "already exists") — it must
/// not be misreported as a name/slug collision.
#[tokio::test]
async fn create_group_400_without_already_exists_body_maps_to_api_error() {
    let mock = AuthentikMock::start().await;
    mock.mock_create_group(
        400,
        serde_json::json!({"name": ["This field is required."]}),
    )
    .await;

    let err = gateway(&mock)
        .create_group(&group("weebo-user"))
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::Api(_)));
}

#[tokio::test]
async fn delete_group_on_404_is_idempotent() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/core/groups/missing/", 404).await;

    let result = gateway(&mock).delete_group("missing").await;
    assert!(result.is_ok());
}

/// A flow spec exercising the non-default arm of every enum-mapping helper
/// in `authentik_http/flows.rs` — the request must serialize without panic
/// and round-trip the create.
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
            designation: FlowDesignation::StageConfiguration,
            authentication: Some(FlowAuthentication::RequireOutpost),
            policy_engine_mode: Some(PolicyEngineMode::All),
            compatibility_mode: Some(true),
            layout: Some(FlowLayout::SidebarLeftFrameBackground),
            denied_action: Some(FlowDeniedAction::MessageContinue),
            background: Some("/static/bg.jpg".to_string()),
        },
        status: None,
    }
}

fn flow_response(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "pk": "33333333-3333-3333-3333-333333333333",
        "policybindingmodel_ptr_id": "44444444-4444-4444-4444-444444444444",
        "name": slug,
        "slug": slug,
        "title": "Device code",
        "designation": "stage_configuration",
        "background_url": "/static/bg.jpg",
        "background_themed_urls": null,
        "stages": [],
        "policies": [],
        "cache_count": 0,
        "export_url": format!("/api/v3/flows/instances/{slug}/export/"),
    })
}

#[tokio::test]
async fn create_flow_success_returns_the_slug() {
    let mock = AuthentikMock::start().await;
    // Flows are slug-keyed: the stored identity is the response slug, not
    // the `pk` UUID.
    mock.mock_post("/flows/instances/", 201, flow_response("device-code"))
        .await;

    let result = gateway(&mock).create_flow(&flow("device-code")).await;
    assert_eq!(result.unwrap(), "device-code");
}

#[tokio::test]
async fn create_flow_conflict_maps_to_already_exists() {
    let mock = AuthentikMock::start().await;
    mock.mock_post(
        "/flows/instances/",
        409,
        serde_json::json!({"slug": ["flow with this slug already exists."]}),
    )
    .await;

    let result = gateway(&mock).create_flow(&flow("device-code")).await;
    assert!(matches!(result, Err(GatewayError::AlreadyExists(_))));
}

#[tokio::test]
async fn delete_flow_on_404_is_idempotent() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/flows/instances/missing/", 404).await;

    let result = gateway(&mock).delete_flow("missing").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn get_application_success_with_no_provider_returns_empty_remote_application() {
    let mock = AuthentikMock::start().await;
    mock.mock_get_application(
        "harbor",
        200,
        serde_json::json!({
            "pk": "22222222-2222-2222-2222-222222222222",
            "name": "Harbor", "slug": "harbor",
            "provider_obj": null, "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;

    let remote = gateway(&mock).get_application("harbor").await.unwrap();
    assert_eq!(remote.provider_id, None);
    assert_eq!(remote.provider_meta_model_name, None);
}

#[tokio::test]
async fn get_application_success_with_a_provider_extracts_its_id_and_kind() {
    let mock = AuthentikMock::start().await;
    mock.mock_get_application(
        "harbor",
        200,
        serde_json::json!({
            "pk": "22222222-2222-2222-2222-222222222222",
            "name": "Harbor", "slug": "harbor",
            "provider": 5,
            "provider_obj": {
                "pk": 5,
                "name": "harbor-oauth2",
                "authentication_flow": null,
                "authorization_flow": "11111111-1111-1111-1111-111111111111",
                "invalidation_flow": "33333333-3333-3333-3333-333333333333",
                "property_mappings": null,
                "component": "ak-provider-oauth2-form",
                "assigned_application_slug": "harbor",
                "assigned_application_name": "Harbor",
                "assigned_backchannel_application_slug": null,
                "assigned_backchannel_application_name": null,
                "verbose_name": "OAuth2/OpenID Provider",
                "verbose_name_plural": "OAuth2/OpenID Providers",
                "meta_model_name": "authentik_providers_oauth2.oauth2provider",
            },
            "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;

    let remote = gateway(&mock).get_application("harbor").await.unwrap();
    assert_eq!(remote.provider_id, Some(5));
    assert_eq!(
        remote.provider_meta_model_name.as_deref(),
        Some("authentik_providers_oauth2.oauth2provider")
    );
}

#[tokio::test]
async fn get_application_not_found_maps_to_not_found() {
    let mock = AuthentikMock::start().await;
    mock.mock_get_application("missing", 404, serde_json::json!({"detail": "not found"}))
        .await;

    let err = gateway(&mock).get_application("missing").await.unwrap_err();
    assert!(matches!(err, GatewayError::NotFound(_)));
}

#[tokio::test]
async fn delete_application_on_404_is_idempotent() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/core/applications/missing/", 404).await;

    let result = gateway(&mock).delete_application("missing").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn create_outpost_success_returns_the_authentik_pk() {
    let mock = AuthentikMock::start().await;
    let expected_pk = "33333333-3333-3333-3333-333333333333";
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

    let outpost = AuthentikOutpost {
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
    };

    let result = gateway(&mock).create_outpost(&outpost).await;
    assert_eq!(result.unwrap(), expected_pk);
}

#[tokio::test]
async fn delete_outpost_on_404_is_idempotent() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/outposts/instances/missing/", 404).await;

    let result = gateway(&mock).delete_outpost("missing").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn delete_provider_hits_the_oauth2_endpoint_for_oauth2_kind() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/providers/oauth2/5/", 204).await;

    let result = gateway(&mock)
        .delete_provider(5, ProviderKind::Oauth2)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn delete_provider_hits_the_proxy_endpoint_for_proxy_kind() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/providers/proxy/7/", 204).await;

    let result = gateway(&mock).delete_provider(7, ProviderKind::Proxy).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn delete_provider_on_404_is_idempotent() {
    let mock = AuthentikMock::start().await;
    mock.mock_delete("/providers/oauth2/99/", 404).await;

    let result = gateway(&mock)
        .delete_provider(99, ProviderKind::Oauth2)
        .await;
    assert!(result.is_ok());
}

fn brand(domain: &str) -> AuthentikBrand {
    AuthentikBrand {
        metadata: ObjectMeta {
            name: Some("weebo".to_string()),
            ..Default::default()
        },
        spec: AuthentikBrandSpec {
            domain: domain.to_string(),
            default: false,
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
        },
        status: None,
    }
}

#[tokio::test]
async fn create_brand_with_no_flow_refs_skips_flow_resolution_entirely() {
    let mock = AuthentikMock::start().await;
    let expected_pk = "44444444-4444-4444-4444-444444444444";
    // No /flows/instances/ mock registered at all — if the gateway tried
    // to resolve a flow slug it isn't supposed to, wiremock would 404 the
    // unexpected request and this test would fail, proving
    // `resolve_optional_flow(None)` is a true no-op.
    mock.mock_post(
        "/core/brands/",
        201,
        serde_json::json!({"brand_uuid": expected_pk, "domain": "weebo.example.com"}),
    )
    .await;

    let result = gateway(&mock)
        .create_brand(&brand("weebo.example.com"))
        .await;
    assert_eq!(result.unwrap(), expected_pk);
}

#[tokio::test]
async fn get_default_brand_returns_the_current_default() {
    let mock = AuthentikMock::start().await;
    let expected_pk = "55555555-5555-5555-5555-555555555555";
    mock.mock_get(
        "/core/brands/",
        200,
        serde_json::json!({
            "pagination": {
                "next": 0, "previous": 0, "count": 1,
                "current": 1, "total_pages": 1, "start_index": 1, "end_index": 1
            },
            "results": [{"brand_uuid": expected_pk, "domain": "weebo.example.com", "default": true}],
            "autocomplete": {}
        }),
    )
    .await;

    let result = gateway(&mock).get_default_brand().await.unwrap();
    assert_eq!(result, Some(expected_pk.to_string()));
}

#[tokio::test]
async fn get_default_brand_returns_none_when_nothing_is_default() {
    let mock = AuthentikMock::start().await;
    mock.mock_get("/core/brands/", 200, empty_paginated_list())
        .await;

    let result = gateway(&mock).get_default_brand().await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn set_brand_default_patches_the_default_flag() {
    let mock = AuthentikMock::start().await;
    let brand_uuid = "66666666-6666-6666-6666-666666666666";
    mock.mock_patch(
        &format!("/core/brands/{brand_uuid}/"),
        200,
        serde_json::json!({"brand_uuid": brand_uuid, "domain": "weebo.example.com", "default": true}),
    )
    .await;

    let result = gateway(&mock).set_brand_default(brand_uuid, true).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn upsert_oauth2_provider_surfaces_an_unresolved_authorization_flow() {
    let mock = AuthentikMock::start().await;
    // authorization_flow is resolved before invalidation_flow (see
    // `AuthentikHttpGateway::upsert_oauth2_provider`) — an empty list here
    // is enough to exercise the not-found path without needing to script
    // every downstream lookup (invalidation flow, property mappings).
    mock.mock_get("/flows/instances/", 200, empty_paginated_list())
        .await;

    let spec = Oauth2ProviderSpec {
        client_id: None,
        authorization_flow: "default-authorization-flow".to_string(),
        invalidation_flow: "default-invalidation-flow".to_string(),
        signing_key: None,
        allowed_redirect_uris: Vec::<RedirectUri>::new(),
        property_mappings: vec![],
        grant_types: vec![],
    };

    let err = gateway(&mock)
        .upsert_oauth2_provider(None, "harbor", &spec)
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::NotFound(_)));
}
