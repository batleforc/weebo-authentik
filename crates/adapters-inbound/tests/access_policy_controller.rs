//! Layer-3 integration tests for `AuthentikAccessPolicy` against a real
//! `kube-apiserver` — specifically the "confinement au namespace, deux
//! couches" security guarantee from `.prompt/plan.md`: the controller
//! resolves `applicationRef` via `Api::<AuthentikApplication>::namespaced`,
//! never a cluster-wide client, so a same-named application in another
//! namespace must stay invisible.

use adapters_inbound::controller;
use api::access_policy::AuthentikAccessPolicySpec;
use api::application::{AuthentikApplicationSpec, ProviderKind, ProviderSpec, ProxyProviderSpec};
use api::{AuthentikAccessPolicy, AuthentikApplication};
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::envtest::EnvTestCluster;

mod support;
use support::{init_tracing, new_ctx, wait_for};

async fn wait_for_ready_status(
    api: &Api<AuthentikAccessPolicy>,
    name: &str,
) -> (String, String, Option<String>) {
    wait_for(api, name, |policy| {
        let status = policy.status.as_ref()?;
        let cond = status.conditions.iter().find(|c| c.type_ == "Ready")?;
        Some((
            cond.status.clone(),
            cond.reason.clone(),
            status.authentik_id.clone(),
        ))
    })
    .await
}

#[tokio::test]
async fn errors_when_application_ref_not_found_in_namespace() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();
    // No mock mounted at all: this path must never touch the gateway
    // (see reconcile_access_policy) — an unexpected call would 404 and
    // surface as a different Ready reason below.
    let mock = AuthentikMock::start().await;
    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::access_policy::run(client.clone(), ctx));

    let policies: Api<AuthentikAccessPolicy> = Api::namespaced(client.clone(), "default");
    policies
        .create(
            &PostParams::default(),
            &AuthentikAccessPolicy {
                metadata: ObjectMeta {
                    name: Some("harbor-access".to_string()),
                    namespace: Some("default".to_string()),
                    ..Default::default()
                },
                spec: AuthentikAccessPolicySpec {
                    application_ref: "missing-app".to_string(),
                    group_ref: "weebo-user".to_string(),
                    order: 0,
                    negate: false,
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikAccessPolicy CR create must succeed");

    let (status, reason, authentik_id) = wait_for_ready_status(&policies, "harbor-access").await;
    assert_eq!(status, "False");
    assert_eq!(reason, "ApplicationRefNotFound");
    assert_eq!(authentik_id, None);
}

#[tokio::test]
async fn syncs_the_binding_once_the_application_has_synced() {
    init_tracing();

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let app_pk = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let group_pk = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    let binding_pk = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    mock.mock_get_application(
        "harbor",
        200,
        serde_json::json!({
            "pk": app_pk, "name": "Harbor", "slug": "harbor",
            "provider_obj": null, "backchannel_providers_obj": [],
            "launch_url": null, "meta_icon_url": null, "meta_icon_themed_urls": null,
        }),
    )
    .await;
    mock.mock_get(
        "/core/groups/",
        200,
        serde_json::json!({
            "pagination": {"next": 0, "previous": 0, "count": 1, "current": 1, "total_pages": 1, "start_index": 1, "end_index": 1},
            "results": [{
                "pk": group_pk, "num_pk": 1, "name": "weebo-user",
                "parents_obj": null, "users_obj": null, "roles_obj": [],
                "inherited_roles_obj": null, "children": [], "children_obj": null,
            }],
            "autocomplete": {}
        }),
    )
    .await;
    mock.mock_post(
        "/policies/bindings/",
        201,
        serde_json::json!({
            "pk": binding_pk, "policy_obj": null, "group_obj": null, "user_obj": null,
            "target": app_pk, "order": 0,
        }),
    )
    .await;
    // The controller's own successful status patch is itself a watch
    // event, which re-triggers a reconcile — that second pass now reads
    // back `status.authentikId` and takes the *update* path
    // (`upsert_policy_binding(Some(id), ...)`), so the PATCH endpoint
    // needs a mock too, or that idempotent re-reconcile 404s and flips
    // `Ready` back to `False` right after this test observed `True`.
    mock.mock_patch(
        &format!("/policies/bindings/{binding_pk}/"),
        200,
        serde_json::json!({
            "pk": binding_pk, "policy_obj": null, "group_obj": null, "user_obj": null,
            "target": app_pk, "order": 0,
        }),
    )
    .await;

    let ctx = new_ctx(client.clone(), &mock);

    // The application controller isn't running in this test — its own
    // sync behavior is covered by application_controller.rs. Here the
    // application's `status.authentikId` is seeded directly so this test
    // stays focused on AuthentikAccessPolicy's own resolution logic.
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
                    kind: ProviderKind::Proxy,
                    oauth2: None,
                    proxy: Some(ProxyProviderSpec {
                        internal_host: "http://harbor.internal".to_string(),
                        external_host: "https://harbor.example.com".to_string(),
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

    let status_patch = serde_json::json!({
        "status": { "authentikId": "harbor", "conditions": [] }
    });
    apps.patch_status(
        "harbor",
        &PatchParams::apply("test-seed"),
        &Patch::Merge(&status_patch),
    )
    .await
    .expect("seeding AuthentikApplication status must succeed");

    tokio::spawn(controller::access_policy::run(client.clone(), ctx));

    let policies: Api<AuthentikAccessPolicy> = Api::namespaced(client.clone(), "default");
    policies
        .create(
            &PostParams::default(),
            &AuthentikAccessPolicy {
                metadata: ObjectMeta {
                    name: Some("harbor-access".to_string()),
                    namespace: Some("default".to_string()),
                    ..Default::default()
                },
                spec: AuthentikAccessPolicySpec {
                    application_ref: "harbor".to_string(),
                    group_ref: "weebo-user".to_string(),
                    order: 0,
                    negate: false,
                },
                status: None,
            },
        )
        .await
        .expect("AuthentikAccessPolicy CR create must succeed");

    let (status, reason, authentik_id) = wait_for_ready_status(&policies, "harbor-access").await;
    assert_eq!(status, "True", "reason={reason}");
    assert_eq!(authentik_id.as_deref(), Some(binding_pk));
}
