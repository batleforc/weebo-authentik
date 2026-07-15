//! Real `kube-apiserver` + `etcd` via the official `kube-rs` `envtest`
//! crate (wraps `sigs.k8s.io/controller-runtime/pkg/envtest`) — layer 3
//! of `.prompt/plan.md`'s test strategy. CRDs from `deploy/crd/` are
//! installed for real; `AuthentikGateway` is swapped for
//! `testkit::authentik_mock`'s `wiremock` server by the caller (this
//! crate never talks to Authentik itself).
//!
//! **Build requirement**: this crate's `envtest` dependency generates Go
//! bindings via `rust2go`/`bindgen` at build time, which needs a Go
//! toolchain (`mise use go`) and `libclang.so` — the C API, not
//! `libclang-cpp.so`, which is all the `mise` `clang` package ships.
//! `libclang.so` here was obtained by extracting it from an official
//! `LLVM-<ver>-Linux-X64.tar.xz` release
//! (<https://github.com/llvm/llvm-project/releases>) into
//! `~/.local/share/llvm-libclang/`, since there's no root/apt access to
//! install it system-wide and no dedicated mise package provides it.
//! Building anything that depends on `testkit` needs:
//! ```text
//! export LIBCLANG_PATH=~/.local/share/llvm-libclang/<ver>/lib
//! export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/<triple>/<ver>/include -I/usr/include"
//! ```
//! (the second exports the system libc's C headers — `bindgen` uses
//! `libclang.so` to parse C, which needs `stddef.h` et al. from
//! somewhere; the mise `clang` package's own bundled headers work too).

use std::path::Path;

use envtest::Environment;
use kube::Client;

pub struct EnvTestCluster {
    // Kept alive only for its `Drop` impl, which tears down the control
    // plane — never read otherwise.
    _server: envtest::Server,
    client: Client,
}

impl EnvTestCluster {
    /// Panics on any setup failure: a broken test harness should fail
    /// loudly right at the call site, not be threaded through `Result`
    /// in every integration test that uses it.
    pub async fn start() -> Self {
        // Both `ring` and `aws-lc-rs` end up linked transitively (`kube`
        // vs. `authentik-client`'s reqwest stack) with no single default
        // rustls `CryptoProvider` — every consumer of this harness needs
        // one picked before its first TLS connection, so it's done once,
        // here, rather than in every test. `install_default` errors if
        // already installed (e.g. a second `EnvTestCluster` in the same
        // process) — ignored, that just means an earlier call won.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let crd_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/crd")
            .canonicalize()
            .expect("deploy/crd must exist — run `task recu` if it's missing");

        let server = Environment::default()
            .with_crds_from_paths(vec![crd_dir.display().to_string()])
            .expect("deploy/crd must be readable")
            .create()
            .await
            .expect("envtest control plane failed to start");
        let client = server.client().expect("envtest client");

        Self {
            _server: server,
            client,
        }
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }
}
