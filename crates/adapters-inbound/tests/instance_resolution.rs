//! Layer-3 integration test of `spec.instanceRef` against a real
//! `kube-apiserver`, with the real instance resolution (the kit's
//! `Instances`): an `AuthentikApplication` bound to an
//! `AuthentikLocalInstance` of its namespace is not governed by the
//! `AuthentikNamespacePolicy` (its namespace's own token bounds it) and
//! records its instance in `status.instance`; one bound to a cluster
//! `AuthentikInstance` still is.

use std::sync::Arc;

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{
    AuthentikHttpGateway, AuthentikInstances, KubePolicySource, kubernetes_secret_store,
};
use api::application::{
    AuthentikApplicationSpec, InstanceKind, InstanceRef, ProviderKind, ProviderSpec,
    ProxyProviderSpec,
};
use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
use api::local_instance::{AuthentikLocalInstanceSpec, LocalSecretKeyRef};
use api::namespace_policy::{AuthentikNamespacePolicySpec, Effect, NamespaceRule, ResourceKind};
use api::status::BoundInstance;
use api::{
    AuthentikApplication, AuthentikInstance, AuthentikLocalInstance, AuthentikNamespacePolicy,
};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::authentik_mock::AuthentikMock;
use testkit::static_gateway_factory::StaticGatewayFactory;
use testkit::static_secret_store_factory::StaticSecretStoreFactory;

mod support;
use support::{init_tracing, wait_for};

fn application(name: &str, instance_ref: InstanceRef) -> AuthentikApplication {
    AuthentikApplication {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: AuthentikApplicationSpec {
            instance_ref,
            name: name.to_string(),
            slug: name.to_string(),
            meta_icon: None,
            provider: ProviderSpec {
                kind: ProviderKind::Proxy,
                oauth2: None,
                proxy: Some(ProxyProviderSpec {
                    internal_host: "http://app.internal".to_string(),
                    external_host: "https://app.example.com".to_string(),
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

#[tokio::test]
async fn a_local_instance_binds_its_applications_outside_the_namespace_policy() {
    init_tracing();
    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    // No mock mounted: an application that gets past the policy fails at
    // its first Authentik call, which is all this test needs to tell it
    // apart from a refused one.
    let mock = AuthentikMock::start().await;
    let gateway = AuthentikHttpGateway::new(
        format!("{}/api/v3", mock.base_path()),
        mock.base_path(),
        "test-token",
    );
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateway_factory: Arc::new(StaticGatewayFactory::new(Arc::new(gateway))),
        secrets_factory: Arc::new(StaticSecretStoreFactory::new(Arc::new(
            kubernetes_secret_store(client.clone()),
        ))),
        instances: Arc::new(AuthentikInstances::new(client.clone())),
        policies: Arc::new(KubePolicySource::new(client.clone())),
        recorder: kube::runtime::events::Recorder::new(
            client.clone(),
            kube::runtime::events::Reporter {
                controller: "weebo-authentik-operator".into(),
                instance: None,
            },
        ),
        shutdown: adapters_inbound::controller::Shutdown::never(),
        health: Arc::new(weebo_kit_runtime::Health::default()),
        scope: weebo_kit_runtime::WatchScope::cluster(),
    });

    let pp = PostParams::default();
    // A policy that allows no AuthentikApplication in `default`.
    Api::<AuthentikNamespacePolicy>::all(client.clone())
        .create(
            &pp,
            &AuthentikNamespacePolicy::new(
                "access-policies-only",
                AuthentikNamespacePolicySpec {
                    rules: vec![NamespaceRule {
                        namespaces: vec!["default".to_string()],
                        allowed_kinds: vec![ResourceKind::AuthentikAccessPolicy],
                        effect: Effect::Allow,
                    }],
                },
            ),
        )
        .await
        .unwrap();
    Api::<AuthentikInstance>::all(client.clone())
        .create(
            &pp,
            &AuthentikInstance::new(
                "prod",
                AuthentikInstanceSpec {
                    url: mock.base_path(),
                    token_secret_ref: SecretKeyRef {
                        name: "token".into(),
                        namespace: "default".into(),
                        key: "token".into(),
                    },
                    tls: Default::default(),
                    secret_store: Default::default(),
                },
            ),
        )
        .await
        .unwrap();
    Api::<AuthentikLocalInstance>::namespaced(client.clone(), "default")
        .create(
            &pp,
            &AuthentikLocalInstance::new(
                "own",
                AuthentikLocalInstanceSpec {
                    url: mock.base_path(),
                    token_secret_ref: LocalSecretKeyRef {
                        name: "token".into(),
                        key: "token".into(),
                    },
                    tls: Default::default(),
                    shared_with: None,
                },
            ),
        )
        .await
        .unwrap();

    tokio::spawn(controller::app::run(client.clone(), ctx));
    let apps: Api<AuthentikApplication> = Api::namespaced(client.clone(), "default");
    apps.create(&pp, &application("on-local", InstanceRef::named("own")))
        .await
        .unwrap();
    let cluster_ref = InstanceRef {
        kind: Some(InstanceKind::Instance),
        ..InstanceRef::named("prod")
    };
    apps.create(&pp, &application("on-cluster", cluster_ref))
        .await
        .unwrap();

    // Bound to the local instance, past the policy: it reached Authentik.
    let (bound, reason) = wait_for(&apps, "on-local", |app| {
        let status = app.status.as_ref()?;
        let ready = weebo_kit_api::status::ready(&status.conditions)?;
        Some((status.instance.clone()?, ready.reason.clone()))
    })
    .await;
    assert_eq!(bound, BoundInstance::local("default", "own"));
    assert_ne!(reason, "NamespaceNotAllowed");

    // Bound to the cluster instance: the policy still refuses it.
    let (bound, reason) = wait_for(&apps, "on-cluster", |app| {
        let status = app.status.as_ref()?;
        let ready = weebo_kit_api::status::ready(&status.conditions)?;
        Some((status.instance.clone()?, ready.reason.clone()))
    })
    .await;
    assert_eq!(bound, BoundInstance::cluster("prod"));
    assert_eq!(reason, "NamespaceNotAllowed");
}
