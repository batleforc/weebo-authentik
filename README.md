# Weebo Authentik

[![CI](https://github.com/batleforc/weebo-authentik/actions/workflows/ci.yml/badge.svg)](https://github.com/batleforc/weebo-authentik/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/batleforc/weebo-authentik/blob/main/Cargo.toml)

A Kubernetes operator that manages [Authentik](https://goauthentik.io/) resources
as native CRDs — groups, users, applications, access policies, brands, and
outposts — replacing a hand-maintained Terraform module with reconciled,
GitOps-friendly Kubernetes objects.

## What it manages

| CRD | Scope | Purpose |
| --- | --- | --- |
| `AuthentikInstance` | Cluster | Points at an Authentik API + bootstrap token |
| `AuthentikGroup` | Cluster | Group hierarchy (`parentRef`), `isSuperuser` |
| `AuthentikUser` | Cluster | Users and their group memberships |
| `AuthentikApplication` | Namespace | An application + its `oauth2` or `proxy` provider |
| `AuthentikAccessPolicy` | Namespace | Binds a group to an application |
| `AuthentikBrand` | Cluster | Domain + branding, with default-brand election |
| `AuthentikOutpost` | Cluster | Outpost config for proxy providers |
| `AuthentikNamespacePolicy` | Namespace | Opts a namespace into the admission-webhook allow-list |

An admission webhook enforces a namespace-scoped allow-list: an
`AuthentikApplication` can only be created in a namespace that has opted in
via `AuthentikNamespacePolicy` (default deny).

Full field-level reference for every CRD is generated into
[`docs/content/docs/crds/`](docs/content/docs/crds) — browse it locally with
`task docs`, or read the `.mdx` files directly on GitHub.

## Quickstart

```bash
# cert-manager is a prerequisite (the webhook's TLS cert is issued by it)
helm install weebo-authentik oci://ghcr.io/batleforc/charts/weebo-authentik \
  --set certManager.issuerRef.name=<your-issuer> \
  --set certManager.issuerRef.kind=ClusterIssuer
```

See [`docs/content/docs/guides/install.mdx`](docs/content/docs/guides/install.mdx)
for the full walkthrough, and
[`docs/content/docs/guides/first-application.mdx`](docs/content/docs/guides/first-application.mdx)
for connecting an `AuthentikInstance` and standing up your first application.

## Architecture

Hexagonal, split by crate:

- `crates/domain` — pure logic (allow-list evaluation, brand-default
  election, status/condition model). No `kube`/`http` dependency.
- `crates/application` — reconcile use-cases orchestrating `domain` +
  the `AuthentikGateway`/`SecretStore` port traits.
- `crates/api` — the CRD types (`kube::CustomResource` derives).
- `crates/adapters-inbound` — the kube.rs controllers + admission webhook.
- `crates/adapters-outbound` — the real Authentik HTTP client, `K8sSecretStore`,
  `VaultSecretStore`.
- `crates/operator` — the binary: wires everything, leader election, webhook TLS.
- `crates/importer` — one-shot tool that reads a live Authentik instance and
  emits CRD YAML (migration path from the Terraform module this replaces).
- `crates/testkit` — shared test harnesses (`envtest`, a `wiremock`-backed
  Authentik mock, generic status-polling helpers).

See [`CLAUDE.md`](CLAUDE.md) for the full crate index and
[`.prompt/plan.md`](.prompt/plan.md) for the authoritative design doc (CRD
scope, error-code conventions, status model, test strategy, security/deployment
decisions) — read it before making any non-trivial change.

## Developing

Requires [`mise`](https://mise.jdx.dev/) — it manages every tool below
(Rust, Go, `task`, `helm`, `kind`, `cocogitto`, ...) pinned in `mise.toml`.

```bash
mise install   # installs every tool this repo needs, pinned in mise.toml
task init      # mise upgrade + installs the cocogitto commit/pre-commit git hooks
task doctor    # sanity-checks the result and prints suggested next steps
```

| Command | What it does |
| --- | --- |
| `task lint` | `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` |
| `task lint:helm` | `helm lint` + a template smoke-test on `deploy/charts/` |
| `task test` | Fast unit + contract tests (`--lib --bins`, no real cluster) |
| `task test:integration` | + real `envtest`-backed controller tests (spins up a real `kube-apiserver`+`etcd`) |
| `task test:all` | `test` + `test:integration` |
| `task recu` | Regenerates CRDs (`deploy/crd/`), the Helm chart's `crds/`, and the CRD docs |
| `task docs` | Runs the Fumadocs documentation site in dev mode |

`crates/testkit`'s `envtest` dependency needs a Go toolchain and a real
`libclang.so` on `LIBCLANG_PATH` for its bindgen build (pulled in by any
build of `adapters-inbound`'s tests, including `clippy --all-targets`) — see
`crates/testkit/src/envtest.rs` and this repo's `Taskfile.yaml` `env:` block
for the documented default, and `task doctor` to check your machine matches it.

## License

[MIT](https://github.com/batleforc/weebo-authentik/blob/main/Cargo.toml)
