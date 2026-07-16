# syntax=docker/dockerfile:1

# The whole workspace is rustls-based end to end (kube's rustls-tls
# default, vaultrs's `default = ["rustls"]`, axum-server's
# tls-rustls-no-provider) — no OpenSSL/libssl needed at build or run
# time. `testkit`/`envtest` (the Go/bindgen toolchain requirement
# documented in crates/testkit/src/envtest.rs) is a [dev-dependencies]
# entry of `operator`, never compiled by a plain `cargo build --release`
# — this image needs nothing beyond a standard Rust toolchain.
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p operator

# distroless/cc matches `rust:*-bookworm`'s glibc ABI (both Debian
# bookworm-based) and already ships ca-certificates + a non-root
# "nonroot" user — no shell, no package manager, minimal attack surface
# for a network-facing admission webhook.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /build/target/release/operator /usr/local/bin/operator
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/operator"]
