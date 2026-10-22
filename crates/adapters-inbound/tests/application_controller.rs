//! Layer-3 integration tests for `AuthentikApplication` against a real
//! `kube-apiserver`: the oauth2 variant proves the whole chain down to a
//! real K8s `Secret` being written (`SecretStore`), the proxy variant
//! proves outpost attachment and — just as importantly — that no
//! `Secret` is ever written for it (see `.prompt/plan.md`: proxy
//! providers carry no client secret).

#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failed step fails the test, with its message"
)]

use adapters_inbound::controller;
use api::AuthentikApplication;
use api::application::{
    AuthentikApplicationSpec, Oauth2ClientType, Oauth2ProviderSpec, ProviderKind, ProviderSpec,
    ProxyProviderSpec,
};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, DeleteParams, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;

mod support;
use support::{init_tracing, new_ctx, wait_for, wait_for_absence};

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

    let cluster = testkit::envtest::start().await;
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
                instance_ref: api::application::InstanceRef::named("prod"),
                name: "Harbor".to_string(),
                slug: "harbor".to_string(),
                meta_icon: None,
                provider: ProviderSpec {
                    kind: ProviderKind::Oauth2,
                    oauth2: Some(Oauth2ProviderSpec {
                        client_type: Oauth2ClientType::Confidential,
                        client_id: None,
                        authorization_flow: "default-authorization-flow".to_string(),
                        invalidation_flow: "default-invalidation-flow".to_string(),
                        signing_key: None,
                        allowed_redirect_uris: vec![],
                        property_mappings: vec![],
                        grant_types: vec![],
                        access_token_validity: None,
                        issuer_url: None,
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

    let cluster = testkit::envtest::start().await;
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
                instance_ref: api::application::InstanceRef::named("prod"),
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

/// The mocks a proxy application `longhorn` needs to sync (provider 43,
/// attached to the embedded outpost).
async fn mount_proxy_sync(mock: &AuthentikMock) {
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
    let outpost = serde_json::json!({
        "pk": outpost_pk, "name": "authentik Embedded Outpost", "type": "proxy",
        "providers": [43], "providers_obj": [], "service_connection_obj": null,
        "refresh_interval_s": 86400, "token_identifier": "tok", "config": {},
    });
    mock.mock_get(
        "/outposts/instances/",
        200,
        serde_json::json!({
            "pagination": {"next": 0, "previous": 0, "count": 1, "current": 1, "total_pages": 1, "start_index": 1, "end_index": 1},
            "results": [outpost.clone()],
            "autocomplete": {}
        }),
    )
    .await;
    mock.mock_get(&format!("/outposts/instances/{outpost_pk}/"), 200, outpost)
        .await;
}

fn proxy_application() -> AuthentikApplication {
    AuthentikApplication {
        metadata: ObjectMeta {
            name: Some("longhorn".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: AuthentikApplicationSpec {
            instance_ref: api::application::InstanceRef::named("prod"),
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
    }
}

async fn deletes(mock: &AuthentikMock) -> Vec<String> {
    mock.server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "DELETE")
        .map(|r| r.url.path().to_string())
        .collect()
}

#[tokio::test]
async fn deleting_an_application_deletes_its_provider_then_the_application() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    mount_proxy_sync(&mock).await;
    mock.mock_get_application(
        "longhorn",
        200,
        serde_json::json!({
            "pk": "66666666-6666-6666-6666-666666666666", "name": "Longhorn", "slug": "longhorn",
            "provider": 43,
            "provider_obj": {
                "pk": 43, "name": "longhorn", "authentication_flow": null,
                "authorization_flow": "d0000000-0000-0000-0000-000000000001",
                "invalidation_flow": "d0000000-0000-0000-0000-000000000002",
                "property_mappings": null, "component": "ak-provider-proxy-form",
                "assigned_application_slug": "longhorn", "assigned_application_name": "Longhorn",
                "assigned_backchannel_application_slug": null,
                "assigned_backchannel_application_name": null,
                "verbose_name": "Proxy Provider", "verbose_name_plural": "Proxy Providers",
                "meta_model_name": "authentik_providers_proxy.proxyprovider",
            },
            "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;
    mock.mock_delete("/providers/proxy/43/", 204).await;
    mock.mock_delete("/core/applications/longhorn/", 204).await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&PostParams::default(), &proxy_application())
        .await
        .expect("AuthentikApplication CR create must succeed");
    wait_for_authentik_id(&apps, "longhorn").await;

    apps.delete("longhorn", &DeleteParams::default())
        .await
        .expect("deleting the application must be accepted");
    wait_for_absence(&apps, "longhorn").await;

    assert_eq!(
        deletes(&mock).await,
        vec![
            "/api/v3/providers/proxy/43/".to_string(),
            "/api/v3/core/applications/longhorn/".to_string(),
        ]
    );
}

/// Deleted by hand in Authentik: the CR is still released, not stuck in
/// Terminating behind a 404, and the provider recorded in its status
/// (`providerId`), left behind by the application, is deleted with it —
/// whatever its kind, the spec may name another one since.
#[tokio::test]
async fn an_application_deleted_out_of_band_is_still_released() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    mount_proxy_sync(&mock).await;
    mock.mock_get_application("longhorn", 404, serde_json::json!({"detail": "Not found."}))
        .await;
    // The recreated application reuses the recorded provider.
    // An update reads the provider first, and writes only what drifted.
    mock.mock_get("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    mock.mock_patch("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    mock.mock_delete("/providers/all/43/", 204).await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&PostParams::default(), &proxy_application())
        .await
        .expect("AuthentikApplication CR create must succeed");
    wait_for_authentik_id(&apps, "longhorn").await;
    let provider_id = wait_for(&apps, "longhorn", |app| {
        app.status
            .as_ref()
            .and_then(|s| s.provider_id.as_deref())
            .map(str::to_string)
    })
    .await;
    assert_eq!(provider_id, "43");

    apps.delete("longhorn", &DeleteParams::default())
        .await
        .expect("deleting the application must be accepted");
    wait_for_absence(&apps, "longhorn").await;
    assert_eq!(
        deletes(&mock).await,
        vec!["/api/v3/providers/all/43/".to_string()]
    );
}

/// Repointed out of band to a provider of a kind the operator never
/// creates: that one is left alone (it isn't this CR's), the recorded one,
/// left behind by the application, is deleted with it.
#[tokio::test]
async fn an_application_repointed_out_of_band_still_deletes_its_recorded_provider() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    mount_proxy_sync(&mock).await;
    mock.mock_get_application(
        "longhorn",
        200,
        serde_json::json!({
            "pk": "66666666-6666-6666-6666-666666666666", "name": "Longhorn", "slug": "longhorn",
            "provider": 99,
            "provider_obj": {
                "pk": 99, "name": "longhorn-saml", "authentication_flow": null,
                "authorization_flow": "d0000000-0000-0000-0000-000000000001",
                "invalidation_flow": "d0000000-0000-0000-0000-000000000002",
                "property_mappings": null, "component": "ak-provider-saml-form",
                "assigned_application_slug": "longhorn", "assigned_application_name": "Longhorn",
                "assigned_backchannel_application_slug": null,
                "assigned_backchannel_application_name": null,
                "verbose_name": "SAML Provider", "verbose_name_plural": "SAML Providers",
                "meta_model_name": "authentik_providers_saml.samlprovider",
            },
            "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;
    mock.mock_delete("/providers/all/43/", 204).await;
    mock.mock_delete("/core/applications/longhorn/", 204).await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&PostParams::default(), &proxy_application())
        .await
        .expect("AuthentikApplication CR create must succeed");
    wait_for_authentik_id(&apps, "longhorn").await;
    let provider_id = wait_for(&apps, "longhorn", |app| {
        app.status
            .as_ref()
            .and_then(|s| s.provider_id.as_deref())
            .map(str::to_string)
    })
    .await;
    assert_eq!(provider_id, "43");

    apps.delete("longhorn", &DeleteParams::default())
        .await
        .expect("deleting the application must be accepted");
    wait_for_absence(&apps, "longhorn").await;
    assert_eq!(
        deletes(&mock).await,
        vec![
            "/api/v3/providers/all/43/".to_string(),
            "/api/v3/core/applications/longhorn/".to_string(),
        ]
    );
}

/// The proxy provider 43, as Authentik returns it.
fn proxy_provider_43() -> serde_json::Value {
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
    })
}

async fn posts(mock: &AuthentikMock, path: &str) -> usize {
    mock.server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == path)
        .count()
}

fn ready_reason(app: &AuthentikApplication) -> Option<String> {
    app.status
        .as_ref()?
        .conditions
        .iter()
        .find(|c| c.type_ == "Ready" && c.status == "False")
        .map(|c| c.reason.clone())
}

/// Created in Authentik, then the outpost attachment fails: the application
/// and its provider are recorded all the same, and the next attempts update
/// them — never a second create colliding with the first.
#[tokio::test]
async fn an_application_created_before_a_failing_step_is_recorded() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    mock.mock_get("/flows/instances/", 200, flows_list()).await;
    mock.mock_post("/providers/proxy/", 201, proxy_provider_43())
        .await;
    // An update reads the provider first, and writes only what drifted.
    mock.mock_get("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    mock.mock_patch("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    let application = serde_json::json!({
        "pk": "66666666-6666-6666-6666-666666666666", "name": "Longhorn", "slug": "longhorn",
        "provider": 43, "provider_obj": null, "backchannel_providers_obj": [],
        "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
    });
    mock.mock_post("/core/applications/", 201, application.clone())
        .await;
    mock.mock_patch("/core/applications/longhorn/", 200, application)
        .await;
    mock.mock_get_application(
        "longhorn",
        200,
        serde_json::json!({
            "pk": "66666666-6666-6666-6666-666666666666", "name": "Longhorn", "slug": "longhorn",
            "provider": 43, "provider_obj": proxy_provider_43(), "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;
    // The embedded outpost can't be listed: the last step fails.
    mock.mock_get(
        "/outposts/instances/",
        500,
        serde_json::json!({"detail": "down"}),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&PostParams::default(), &proxy_application())
        .await
        .expect("AuthentikApplication CR create must succeed");

    let (id, provider) = wait_for(&apps, "longhorn", |app| {
        ready_reason(app)?;
        let status = app.status.as_ref()?;
        Some((status.authentik_id.clone()?, status.provider_id.clone()?))
    })
    .await;
    assert_eq!((id.as_str(), provider.as_str()), ("longhorn", "43"));

    // Further attempts (the failure requeues) update what was recorded.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_eq!(posts(&mock, "/api/v3/core/applications/").await, 1);
    assert_eq!(posts(&mock, "/api/v3/providers/proxy/").await, 1);
}

/// The application can't be created (its slug is taken), the provider made
/// for it can: recorded, it is patched by the next attempts rather than
/// created again, and deleted with the CR.
#[tokio::test]
async fn a_provider_recorded_without_its_application_is_deleted_with_the_cr() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    mock.mock_get("/flows/instances/", 200, flows_list()).await;
    mock.mock_post("/providers/proxy/", 201, proxy_provider_43())
        .await;
    // An update reads the provider first, and writes only what drifted.
    mock.mock_get("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    mock.mock_patch("/providers/proxy/43/", 200, proxy_provider_43())
        .await;
    mock.mock_post(
        "/core/applications/",
        400,
        serde_json::json!({"slug": ["Application with this slug already exists."]}),
    )
    .await;
    mock.mock_delete("/providers/all/43/", 204).await;

    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::app::run(client.clone(), ctx));

    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&PostParams::default(), &proxy_application())
        .await
        .expect("AuthentikApplication CR create must succeed");

    let provider = wait_for(&apps, "longhorn", |app| {
        app.status.as_ref()?.provider_id.clone()
    })
    .await;
    assert_eq!(provider, "43");
    let recorded = apps.get("longhorn").await.unwrap();
    assert_eq!(
        ready_reason(&recorded).as_deref(),
        Some("AuthentikObjectAlreadyExists")
    );
    assert_eq!(recorded.status.unwrap().authentik_id, None);

    apps.delete("longhorn", &DeleteParams::default())
        .await
        .expect("deleting the application must be accepted");
    wait_for_absence(&apps, "longhorn").await;
    assert_eq!(posts(&mock, "/api/v3/providers/proxy/").await, 1);
    assert_eq!(
        deletes(&mock).await,
        vec!["/api/v3/providers/all/43/".to_string()]
    );
}
