//! Shared boilerplate for the layer-3 integration tests in this crate's
//! `tests/` directory (`group_controller.rs` is the documented template
//! every other CRD's integration test copies from). A `tests/support/mod.rs`
//! is not itself compiled as a test binary — `cargo` only builds it when
//! another test file does `mod support;` — so this is the right place to
//! consolidate the Ctx-construction and status-polling patterns that used
//! to be copy-pasted (and, in two files, independently but incompletely
//! extracted) across every `*_controller.rs`.
//!
//! Not every test binary uses every helper here, hence the blanket
//! `allow(dead_code)`: per-binary dead-code warnings would otherwise trip
//! `cargo clippy -D warnings` for whichever helpers a given file doesn't
//! happen to call.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::Ctx;
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use testkit::authentik_mock::AuthentikMock;
use testkit::static_gateway_factory::StaticGatewayFactory;

/// `let _ = tracing_subscriber::fmt().with_test_writer().try_init();` —
/// idempotent across the many `#[tokio::test]` fns in a single binary
/// (`try_init` no-ops after the first call), so every test can call this
/// unconditionally.
pub fn init_tracing() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}

/// Builds the `Ctx` every controller integration test wires up: a real
/// `AuthentikHttpGateway` pointed at the test's `AuthentikMock` (wrapped in
/// a `StaticGatewayFactory` so no real `AuthentikInstance` resolution is
/// needed), and a real `K8sSecretStore` against the test's `envtest`
/// client.
pub fn new_ctx(client: kube::Client, mock: &AuthentikMock) -> Arc<Ctx> {
    let gateway = AuthentikHttpGateway::new(format!("{}/api/v3", mock.base_path()), "test-token");
    let gateway_factory = Arc::new(StaticGatewayFactory::new(Arc::new(gateway)));
    let secrets = Arc::new(K8sSecretStore::new(client.clone()));
    Arc::new(Ctx {
        client,
        gateway_factory,
        secrets,
    })
}

/// Polls `api.get(name)` every 200ms until `extract` returns `Some`, up to
/// a 15s timeout — the shape every ad-hoc `tokio::time::timeout` polling
/// loop in this crate's integration tests used, just parameterized over
/// what's being waited for (a status field, a `Ready` condition, ...).
pub async fn wait_for<K, T, F>(api: &kube::Api<K>, name: &str, mut extract: F) -> T
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
    F: FnMut(&K) -> Option<T>,
{
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let obj = api.get(name).await.expect("CR must be gettable");
            if let Some(v) = extract(&obj) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("condition on {name:?} was not met within the 15s timeout"))
}

/// Polls `api.get(name)` every 200ms until it errors (i.e. the object is
/// gone), up to a 15s timeout — the delete-then-wait-for-404 half of the
/// finalizer-cleanup pattern.
pub async fn wait_for_absence<K>(api: &kube::Api<K>, name: &str)
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if api.get(name).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{name:?} was not removed within the 15s timeout"))
}
