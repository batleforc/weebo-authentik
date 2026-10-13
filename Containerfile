# ── Build stage ───────────────────────────────────────────────────────────────
# The whole workspace is rustls-based end to end (kube's rustls-tls
# default, vaultrs's `default = ["rustls"]`, axum-server's
# tls-rustls-no-provider) — no OpenSSL/libssl needed at build or run time.
# `testkit`/`envtest` is a dev-dependency, never compiled by this release
# build: a standard Rust toolchain is all it needs.
FROM rust:1-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS builder

WORKDIR /build

# Cache dependency compilation separately from source changes: manifests
# first, every crate stubbed, deps built, then the real sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN find crates -name '*.rs' -delete && \
    for crate in crates/*/; do \
      mkdir -p "$crate/src" && echo "pub fn _stub() {}" > "$crate/src/lib.rs"; \
    done && \
    for bin in operator crdgen importer; do echo "fn main() {}" > "crates/$bin/src/main.rs"; done

# --locked: build exactly what Cargo.lock resolves — the image must not
# resolve a dependency the repository never saw. Failure is tolerated here
# only because stubs may not satisfy every crate; the real build below is
# the one that gates.
RUN cargo build --locked --release -p operator 2>/dev/null; exit 0

COPY crates/ crates/
RUN find crates -name '*.rs' -exec touch {} + && \
    cargo build --locked --release -p operator

# ── Runtime image ─────────────────────────────────────────────────────────────
# distroless/cc on Debian 13 matches trixie's glibc ABI (the builder's) and
# ships ca-certificates + a non-root "nonroot" user — no shell, no package
# manager, minimal surface for a network-facing admission webhook. Debian
# 12's still carried High libssl3 findings when this moved.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:e792ab3d241a468a4fd7519ddbbebe66b49b5f365771716ea688ad40b6c6f1c2 AS runtime

COPY --from=builder /build/target/release/operator /usr/local/bin/operator

# 65532 is distroless's `nonroot`. Numeric so Kubernetes can enforce
# `runAsNonRoot` without resolving names; the Helm chart pins the same UID.
USER 65532:65532

# Static OCI metadata; version, revision and created are added by the
# publish workflow (build-push-action `labels:`).
LABEL org.opencontainers.image.title="weebo-authentik-operator" \
      org.opencontainers.image.description="Kubernetes operator managing Authentik objects as CRDs" \
      org.opencontainers.image.source="https://github.com/batleforc/weebo-authentik" \
      org.opencontainers.image.url="https://github.com/batleforc/weebo-authentik" \
      org.opencontainers.image.vendor="batleforc"

ENTRYPOINT ["/usr/local/bin/operator"]
