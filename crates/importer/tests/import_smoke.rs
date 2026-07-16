//! Functional smoke test: runs the real compiled `importer` binary
//! against a `wiremock`-scripted Authentik API (a group, a user, one
//! oauth2 application with a policy binding) and confirms the generated
//! YAML files parse back into the real `api::*` CRD types with the
//! expected fields and a non-empty `status.authentikId`.

use std::process::Command;

use api::{AuthentikAccessPolicy, AuthentikApplication, AuthentikGroup, AuthentikUser};
use testkit::authentik_mock::AuthentikMock;

fn paginated(results: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "pagination": {
            "next": 0, "previous": 0, "count": 1, "current": 1,
            "total_pages": 1, "start_index": 1, "end_index": 1,
        },
        "results": results,
        "autocomplete": {},
    })
}

#[tokio::test]
async fn import_generates_valid_crds_with_authentik_id() {
    let mock = AuthentikMock::start().await;

    let group_pk = "11111111-1111-1111-1111-111111111111";
    let flow_pk = "22222222-2222-2222-2222-222222222222";
    let app_pk = "33333333-3333-3333-3333-333333333333";
    let binding_pk = "44444444-4444-4444-4444-444444444444";

    mock.mock_get(
        "/core/groups/",
        200,
        paginated(serde_json::json!([{
            "pk": group_pk, "num_pk": 1, "name": "weebo-user",
            "is_superuser": false, "parents": [], "parents_obj": null,
            "users": [], "users_obj": null, "attributes": {},
            "roles": [], "roles_obj": [], "inherited_roles_obj": null,
            "children": [], "children_obj": null,
        }])),
    )
    .await;

    mock.mock_get(
        "/core/users/",
        200,
        paginated(serde_json::json!([{
            "pk": 1, "username": "alice", "name": "Alice",
            "is_active": true, "date_joined": "2026-01-01T00:00:00Z",
            "is_superuser": false, "groups": [group_pk], "groups_obj": null,
            "roles": [], "roles_obj": null, "email": "alice@example.com",
            "avatar": "", "attributes": {}, "uid": "alice-uid", "path": null,
            "type": "internal", "uuid": "55555555-5555-5555-5555-555555555555",
            "password_change_date": "2026-01-01T00:00:00Z",
            "last_updated": "2026-01-01T00:00:00Z",
        }])),
    )
    .await;

    mock.mock_get("/core/brands/", 200, paginated(serde_json::json!([])))
        .await;
    mock.mock_get(
        "/outposts/instances/",
        200,
        paginated(serde_json::json!([])),
    )
    .await;
    mock.mock_get(
        "/propertymappings/all/",
        200,
        paginated(serde_json::json!([])),
    )
    .await;
    mock.mock_get(
        "/crypto/certificatekeypairs/",
        200,
        paginated(serde_json::json!([])),
    )
    .await;

    mock.mock_get(
        "/flows/instances/",
        200,
        paginated(serde_json::json!([{
            "pk": flow_pk, "policybindingmodel_ptr_id": flow_pk,
            "name": "default-authorization-flow", "slug": "default-authorization-flow",
            "title": "", "designation": "authorization", "background": "",
            "background_url": "", "background_themed_urls": null,
            "stages": [], "policies": [], "cache_count": 0,
            "policy_engine_mode": "any", "compatibility_mode": false,
            "export_url": "", "layout": "stacked", "denied_action": "message_continue",
            "authentication": "require_authenticated",
        }])),
    )
    .await;

    mock.mock_get(
        "/providers/all/",
        200,
        paginated(serde_json::json!([{
            "pk": 100, "name": "harbor", "authorization_flow": flow_pk,
            "invalidation_flow": flow_pk, "component": "ak-provider-oauth2-form",
            "assigned_application_slug": "harbor", "assigned_application_name": "Harbor",
            "assigned_backchannel_application_slug": null,
            "assigned_backchannel_application_name": null,
            "verbose_name": "OAuth2/OpenID Provider", "verbose_name_plural": "OAuth2/OpenID Providers",
            "meta_model_name": "authentik_providers_oauth2.oauth2provider",
        }])),
    )
    .await;

    mock.mock_get(
        "/providers/oauth2/100/",
        200,
        serde_json::json!({
            "pk": 100, "name": "harbor", "authorization_flow": flow_pk,
            "invalidation_flow": flow_pk, "component": "ak-provider-oauth2-form",
            "assigned_application_slug": "harbor", "assigned_application_name": "Harbor",
            "assigned_backchannel_application_slug": null,
            "assigned_backchannel_application_name": null,
            "verbose_name": "OAuth2/OpenID Provider", "verbose_name_plural": "OAuth2/OpenID Providers",
            "meta_model_name": "authentik_providers_oauth2.oauth2provider",
            "client_id": "harbor-client-id",
            "redirect_uris": [{"matching_mode": "strict", "url": "https://harbor.example.com/cb"}],
            "grant_types": ["authorization_code"],
        }),
    )
    .await;

    mock.mock_get(
        "/core/applications/",
        200,
        paginated(serde_json::json!([{
            "pk": app_pk, "name": "Harbor", "slug": "harbor",
            "provider": 100, "provider_obj": null,
            "backchannel_providers": [], "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
            "meta_icon": "harbor.svg",
        }])),
    )
    .await;

    mock.mock_get(
        "/policies/bindings/",
        200,
        paginated(serde_json::json!([{
            "pk": binding_pk, "group": group_pk, "policy_obj": null,
            "group_obj": null, "user_obj": null, "target": app_pk,
            "negate": false, "order": 0,
        }])),
    )
    .await;

    let dir = std::env::temp_dir().join(format!("import-smoke-{}", std::process::id()));

    let status = Command::new(env!("CARGO_BIN_EXE_importer"))
        .args([
            "--authentik-url",
            &format!("{}/api/v3", mock.base_path()),
            "--authentik-token",
            "test-token",
            "--instance-ref",
            "my-instance",
            "--namespace",
            "team-a",
            "--out",
        ])
        .arg(&dir)
        .status()
        .expect("importer binary must spawn");
    assert!(status.success(), "importer must exit successfully");

    let group_cr: AuthentikGroup = serde_norway::from_str(
        &std::fs::read_to_string(dir.join("authentikgroup-weebo-user.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(group_cr.spec.name, "weebo-user");
    assert_eq!(
        group_cr
            .status
            .as_ref()
            .and_then(|s| s.authentik_id.clone()),
        Some(group_pk.to_string())
    );

    let user_cr: AuthentikUser = serde_norway::from_str(
        &std::fs::read_to_string(dir.join("authentikuser-alice.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(user_cr.spec.username, "alice");
    assert_eq!(user_cr.spec.group_refs, vec!["weebo-user".to_string()]);

    let app_cr: AuthentikApplication = serde_norway::from_str(
        &std::fs::read_to_string(dir.join("authentikapplication-harbor.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(app_cr.spec.slug, "harbor");
    assert_eq!(app_cr.spec.instance_ref, "my-instance");
    assert_eq!(app_cr.metadata.namespace.as_deref(), Some("team-a"));
    assert_eq!(
        app_cr.status.as_ref().and_then(|s| s.authentik_id.clone()),
        Some("harbor".to_string())
    );
    let oauth2 = app_cr
        .spec
        .provider
        .oauth2
        .expect("must be an oauth2 provider");
    assert_eq!(oauth2.client_id.as_deref(), Some("harbor-client-id"));
    assert_eq!(oauth2.allowed_redirect_uris.len(), 1);

    let policy_cr: AuthentikAccessPolicy = serde_norway::from_str(
        &std::fs::read_to_string(dir.join("authentikaccesspolicy-harbor-weebo-user.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(policy_cr.spec.application_ref, "harbor");
    assert_eq!(policy_cr.spec.group_ref, "weebo-user");
    assert_eq!(
        policy_cr
            .status
            .as_ref()
            .and_then(|s| s.authentik_id.clone()),
        Some(binding_pk.to_string())
    );

    std::fs::remove_dir_all(&dir).ok();
}
