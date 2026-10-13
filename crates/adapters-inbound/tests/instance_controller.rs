//! Layer-3 integration test: `AuthentikInstance` has no Authentik-side
//! object (see `application::use_cases::reconcile_instance`) — this
//! proves the controller marks an accepted CR `Ready: True` with no
//! `authentikId` and, crucially, without ever calling the gateway (no
//! mock is mounted on the `AuthentikMock` server below; an unexpected
//! call would 404 and this test would fail with a different reason) —
//! and that its finalizer holds it while CRs are bound to it.

use adapters_inbound::controller;
use api::application::{AuthentikApplicationSpec, ProviderKind, ProviderSpec, ProxyProviderSpec};
use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
use api::{AuthentikApplication, AuthentikInstance};
use kube::api::{Api, DeleteParams, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;

mod support;
use support::{init_tracing, new_ctx, wait_for, wait_for_event};

#[tokio::test]
async fn instance_controller_marks_ready_with_no_remote_object() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();

    let mock = AuthentikMock::start().await;
    let ctx = new_ctx(client.clone(), &mock);

    tokio::spawn(controller::instance::run(client.clone(), ctx));

    let instances: Api<AuthentikInstance> = Api::all(client.clone());
    instances
        .create(&PostParams::default(), &instance("prod"))
        .await
        .expect("AuthentikInstance CR create must succeed");

    let authentik_id = wait_for(&instances, "prod", |instance| {
        instance
            .status
            .as_ref()
            .filter(|status| {
                status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .map(|status| status.authentik_id.clone())
    })
    .await;

    assert_eq!(authentik_id, None);
}

fn instance(name: &str) -> AuthentikInstance {
    AuthentikInstance {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        spec: AuthentikInstanceSpec {
            url: "https://login.example.com".to_string(),
            token_secret_ref: SecretKeyRef {
                name: "authentik-token".to_string(),
                namespace: "default".to_string(),
                key: "token".to_string(),
            },
            tls: Default::default(),
            secret_store: Default::default(),
        },
        status: None,
    }
}

/// The instance's finalizer holds its deletion while a CR is bound to it
/// (here: an application naming it, not resolved yet) — that CR's own
/// cleanup needs the instance — and releases it once the CR is gone.
#[tokio::test]
async fn an_instance_in_use_is_kept_until_its_crs_are_gone() {
    init_tracing();

    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let mock = AuthentikMock::start().await;
    let ctx = new_ctx(client.clone(), &mock);
    tokio::spawn(controller::instance::run(client.clone(), ctx));

    let instances: Api<AuthentikInstance> = Api::all(client.clone());
    instances
        .create(&PostParams::default(), &instance("prod"))
        .await
        .expect("AuthentikInstance CR create must succeed");
    // The finalizer is on once it is Ready.
    wait_for(&instances, "prod", |i| {
        i.status
            .as_ref()?
            .conditions
            .iter()
            .any(|c| c.type_ == "Ready" && c.status == "True")
            .then_some(())
    })
    .await;

    // No application controller here: the CR carries no finalizer.
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

    instances
        .delete("prod", &DeleteParams::default())
        .await
        .expect("deleting the instance must be accepted");
    assert_eq!(
        wait_for_event(&client, "default", "prod", "InstanceInUse").await,
        "Warning"
    );
    assert!(
        instances.get_opt("prod").await.unwrap().is_some(),
        "the instance is kept while an application uses it"
    );

    apps.delete("longhorn", &DeleteParams::default())
        .await
        .expect("deleting the application must succeed");
    // Nothing watches the application from here: released on the
    // cleanup's next retry (~30s, jittered).
    testkit::polling::wait_for_absence(
        &instances,
        "prod",
        std::time::Duration::from_secs(60),
        std::time::Duration::from_millis(500),
    )
    .await;
}
