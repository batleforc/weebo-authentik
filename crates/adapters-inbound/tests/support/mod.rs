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
use testkit::static_secret_store_factory::StaticSecretStoreFactory;

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
/// client (wrapped in a `StaticSecretStoreFactory`, same rationale — the
/// Vault backend is exercised by its own contract test, not these
/// per-CRD controller tests).
pub fn new_ctx(client: kube::Client, mock: &AuthentikMock) -> Arc<Ctx> {
    let gateway = AuthentikHttpGateway::new(
        format!("{}/api/v3", mock.base_path()),
        mock.base_path(),
        "test-token",
    );
    let gateway_factory = Arc::new(StaticGatewayFactory::new(Arc::new(gateway)));
    let secrets = Arc::new(K8sSecretStore::new(client.clone()));
    let secrets_factory = Arc::new(StaticSecretStoreFactory::new(secrets));
    Arc::new(Ctx {
        client,
        gateway_factory,
        secrets_factory,
    })
}

/// This crate's tuning (15s timeout, 200ms poll interval) of
/// `testkit::polling`'s generic wait helpers, shared with `operator`'s
/// real-sidecar-Authentik tests (which use a longer 30s/300ms budget — see
/// `crates/operator/tests/sidecar_authentik.rs`).
const TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub async fn wait_for<K, T, F>(api: &kube::Api<K>, name: &str, extract: F) -> T
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
    F: FnMut(&K) -> Option<T>,
{
    testkit::polling::wait_for(api, name, TIMEOUT, POLL_INTERVAL, extract).await
}

pub async fn wait_for_absence<K>(api: &kube::Api<K>, name: &str)
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    testkit::polling::wait_for_absence(api, name, TIMEOUT, POLL_INTERVAL).await
}
