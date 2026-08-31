//! Layer-3 integration tests for `AuthentikApplication` against a real
//! `kube-apiserver`: the oauth2 variant proves the whole chain down to a
//! real K8s `Secret` being written (`SecretStore`), the proxy variant
//! proves outpost attachment and — just as importantly — that no
//! `Secret` is ever written for it (see `.prompt/plan.md`: proxy
//! providers carry no client secret).

use adapters_inbound::controller;
use api::AuthentikApplication;
use api::application::{
    AuthentikApplicationSpec, Oauth2ProviderSpec, ProviderKind, ProviderSpec, ProxyProviderSpec,
};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

/// Serves both the authorization_flow and invalidation_flow lookups: a
/// wiremock `path()` match ignores the query string, so one canned
/// response containing both slugs satisfies whichever one is requested.
fn flows_list() -> serde_json::Value {
    serde_json::json!({
        "pagination": {"next": 0, "previous": 0, "count": 2, "current": 1, "total_pages": 1, "start_index": 1, "end_index": 2},
        "results": [
            {
                "pk": "d0000000-0000-0000-0000-000000000001", "policybindingmodel_ptr_id": "d0000000-0000-0000-0000-000000000001",
                "name": "default-authorization-flow", "slug": "default-authorization-flow", "title": "Authorize",
                "designation": "authorization", "background_url": "", "background_themed_urls": null,
                "stages": [], "policies": [], "cache_count": 0, "export_url": "",
            },
            {
                "pk": "d0000000-0000-0000-0000-000000000002", "policybindingmodel_ptr_id": "d0000000-0000-0000-0000-000000000002",
                "name": "default-invalidation-flow", "slug": "default-invalidation-flow", "title": "Invalidate",
                "designation": "invalidation", "background_url": "", "background_themed_urls": null,
                "stages": [], "policies": [], "cache_count": 0, "export_url": "",
            },
        ],
        "autocomplete": {}
    })
}

async fn wait_for_authentik_id(api: &Api<AuthentikApplication>, name: &str) -> String {
    wait_for(api, name, |app| {
        app.status
            .as_ref()
            .and_then(|s| s.authentik_id.as_deref())
            .map(str::to_string)
    })
    .await
}

#[tokio::test]
async fn oauth2_application_syncs_id_and_writes_the_credentials_secret() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_get("/flows/instances/", 200, flows_list()).await;
    mock.mock_post(
        "/providers/oauth2/",
        201,
        serde_json::json!({
            "pk": 42, "name": "harbor", "component": "ak-provider-oauth2-form",
            "authorization_flow": "d0000000-0000-0000-0000-000000000001",
            "invalidation_flow": "d0000000-0000-0000-0000-000000000002",
            "assigned_application_slug": null, "assigned_application_name": null,
            "assigned_backchannel_application_slug": null, "assigned_backchannel_application_name": null,
            "verbose_name": "OAuth2/OIDC Provider", "verbose_name_plural": "OAuth2/OIDC Providers",
            "meta_model_name": "authentik_providers_oauth2.oauth2provider",
            "client_id": "generated-client-id", "client_secret": "generated-client-secret",
            "redirect_uris": [],
        }),
    )
    .await;
    mock.mock_post(
        "/core/applications/",
        201,
        serde_json::json!({
            "pk": "55555555-5555-5555-5555-555555555555", "name": "Harbor", "slug": "harbor",
            "provider_obj": null, "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(
        &PostParams::default(),
        &AuthentikApplication {
            metadata: ObjectMeta {
                name: Some("harbor".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: AuthentikApplicationSpec {
                instance_ref: "prod".to_string(),
                name: "Harbor".to_string(),
                slug: "harbor".to_string(),
                meta_icon: None,
                provider: ProviderSpec {
                    kind: ProviderKind::Oauth2,
                    oauth2: Some(Oauth2ProviderSpec {
                        client_id: None,
                        authorization_flow: "default-authorization-flow".to_string(),
                        invalidation_flow: "default-invalidation-flow".to_string(),
                        signing_key: None,
                        allowed_redirect_uris: vec![],
                        property_mappings: vec![],
                        grant_types: vec![],
                    }),
                    proxy: None,
                },
                secret_targets: Vec::new(),
            },
            status: None,
        },
    )
    .await
    .expect("AuthentikApplication CR create must succeed");

    // `create_application` (AuthentikHttpGateway) returns the *slug*, not
    // the `pk` — Authentik's applications API is slug-keyed (see its own
    // doc comment) — so `status.authentikId` ends up "harbor", not the
    // mocked response's `pk`.
    let synced_id = wait_for_authentik_id(&apps, "harbor").await;
    assert_eq!(synced_id, "harbor");

    let secrets: Api<Secret> = Api::namespaced(client.clone(), "default");
    let secret = secrets
        .get("harbor")
        .await
        .expect("K8sSecretStore must have written a Secret named after the CR");
    let data = secret.data.expect("Secret must carry data");
    assert_eq!(
        data.get("AUTHENTIK_CLIENT_ID").map(|v| v.0.as_slice()),
        Some(b"generated-client-id".as_slice())
    );
    assert_eq!(
        data.get("AUTHENTIK_CLIENT_SECRET").map(|v| v.0.as_slice()),
        Some(b"generated-client-secret".as_slice())
    );
}

#[tokio::test]
async fn proxy_application_syncs_id_attaches_outpost_and_writes_no_secret() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    mock.mock_get("/flows/instances/", 200, flows_list()).await;
    mock.mock_post(
        "/providers/proxy/",
        201,
        serde_json::json!({
            "pk": 43, "name": "longhorn", "component": "ak-provider-proxy-form",
            "authorization_flow": "d0000000-0000-0000-0000-000000000001",
            "invalidation_flow": "d0000000-0000-0000-0000-000000000002",
            "assigned_application_slug": null, "assigned_application_name": null,
            "assigned_backchannel_application_slug": null, "assigned_backchannel_application_name": null,
            "verbose_name": "Proxy Provider", "verbose_name_plural": "Proxy Providers",
            "meta_model_name": "authentik_providers_proxy.proxyprovider",
            "client_id": "embedded-outpost-client-id", "external_host": "https://longhorn.example.com",
            "redirect_uris": [], "outpost_set": [],
        }),
    )
    .await;
    mock.mock_post(
        "/core/applications/",
        201,
        serde_json::json!({
            "pk": "66666666-6666-6666-6666-666666666666", "name": "Longhorn", "slug": "longhorn",
            "provider_obj": null, "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;
    let outpost_pk = "77777777-aaaa-bbbb-cccc-ddddddddddd0";
    mock.mock_get(
        "/outposts/instances/",
        200,
        serde_json::json!({
            "pagination": {"next": 0, "previous": 0, "count": 1, "current": 1, "total_pages": 1, "start_index": 1, "end_index": 1},
            "results": [{
                "pk": outpost_pk, "name": "authentik Embedded Outpost", "type": "proxy",
                "providers": [], "providers_obj": [], "service_connection_obj": null,
                "refresh_interval_s": 86400, "token_identifier": "tok", "config": {},
            }],
            "autocomplete": {}
        }),
    )
    .await;
    // `attach_outpost` reads the outpost's *current* provider list before
    // appending to it (`outposts_instances_retrieve`) — a separate
    // GET-by-id call from the list-by-name lookup just above.
    mock.mock_get(
        &format!("/outposts/instances/{outpost_pk}/"),
        200,
        serde_json::json!({
            "pk": outpost_pk, "name": "authentik Embedded Outpost", "type": "proxy",
            "providers": [], "providers_obj": [], "service_connection_obj": null,
            "refresh_interval_s": 86400, "token_identifier": "tok", "config": {},
        }),
    )
    .await;
    mock.mock_patch(
        &format!("/outposts/instances/{outpost_pk}/"),
        200,
        serde_json::json!({
            "pk": outpost_pk, "name": "authentik Embedded Outpost", "type": "proxy",
            "providers": [43], "providers_obj": [], "service_connection_obj": null,
            "refresh_interval_s": 86400, "token_identifier": "tok", "config": {},
        }),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(
        &PostParams::default(),
        &AuthentikApplication {
            metadata: ObjectMeta {
                name: Some("longhorn".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: AuthentikApplicationSpec {
                instance_ref: "prod".to_string(),
                name: "Longhorn".to_string(),
                slug: "longhorn".to_string(),
                meta_icon: None,
                provider: ProviderSpec {
                    kind: ProviderKind::Proxy,
                    oauth2: None,
                    proxy: Some(ProxyProviderSpec {
                        internal_host: "http://longhorn.internal".to_string(),
                        external_host: "https://longhorn.example.com".to_string(),
                        authorization_flow: "default-authorization-flow".to_string(),
                        invalidation_flow: "default-invalidation-flow".to_string(),
                        outpost_ref: None,
                    }),
                },
                secret_targets: Vec::new(),
            },
            status: None,
        },
    )
    .await
    .expect("AuthentikApplication CR create must succeed");

    let synced_id = wait_for_authentik_id(&apps, "longhorn").await;
    assert_eq!(synced_id, "longhorn");

    let secrets: Api<Secret> = Api::namespaced(client.clone(), "default");
    let result = secrets.get("longhorn").await;
    assert!(
        matches!(result, Err(kube::Error::Api(ref e)) if e.code == 404),
        "a proxy provider must never get a credentials Secret written, got {result:?}"
    );
}
