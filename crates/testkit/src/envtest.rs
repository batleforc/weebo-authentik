//! Real `kube-apiserver` + `etcd` (the kit's `EnvTestCluster`) with this
//! operator's CRDs from `deploy/crd/` installed. `AuthentikGateway` is
//! swapped for `testkit::authentik_mock`'s `wiremock` server by the caller
//! (this crate never talks to Authentik itself).
//!
//! Building it needs Go and `libclang.so` — see `task doctor` and the
//! Taskfile's `env:` block.

use std::path::Path;

pub use weebo_kit_testkit::envtest::EnvTestCluster;

/// Starts a control plane with every CRD of `deploy/crd/` installed
/// (`task recu` regenerates them).
pub async fn start() -> EnvTestCluster {
    EnvTestCluster::start(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/crd")).await
}
