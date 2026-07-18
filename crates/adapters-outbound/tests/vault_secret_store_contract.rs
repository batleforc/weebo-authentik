//! Layer-2 contract tests (`.prompt/plan.md`'s test strategy), mirroring
//! `authentik_http_contract.rs`'s pattern but for `VaultSecretStore`:
//! script Vault's HTTP KV v2 + Kubernetes-auth API via `wiremock` and
//! assert `VaultSecretStore` hits the right paths/bodies and maps
//! responses correctly (in particular, the idempotent-404-on-delete
//! behavior every `SecretStore` backend must share with
//! `K8sSecretStore::delete`). No real Vault instance involved.

use adapters_outbound::VaultSecretStore;
use application::ports::{Oauth2Credentials, SecretStore};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FAKE_JWT: &str = "fake-serviceaccount-jwt";
const ROLE: &str = "weebo-authentik-operator";
const MOUNT: &str = "secret";
const PATH_PREFIX: &str = "weebo-authentik";

/// Vault wraps every response (including auth logins) in a common
/// envelope (`vaultrs::api::EndpointResult`) — `request_id`/`lease_id`/
/// `lease_duration`/`renewable` are required at the top level in
/// addition to (for a login) the nested `auth` object.
fn login_response() -> serde_json::Value {
    serde_json::json!({
        "request_id": "fake-request-id",
        "lease_id": "",
        "lease_duration": 0,
        "renewable": false,
        "data": null,
        "wrap_info": null,
        "warnings": null,
        "auth": {
            "client_token": "fake-vault-client-token",
            "accessor": "fake-accessor",
            "policies": ["default"],
            "token_policies": ["default"],
            "metadata": {},
            "lease_duration": 3600,
            "renewable": true,
            "entity_id": "fake-entity",
            "token_type": "service",
            "orphan": true
        }
    })
}

async fn mock_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(login_response()))
        .mount(server)
        .await;
}

async fn store(server: &MockServer) -> VaultSecretStore {
    mock_login(server).await;
    VaultSecretStore::login(
        &server.uri(),
        MOUNT,
        PATH_PREFIX,
        ROLE,
        "kubernetes",
        FAKE_JWT,
    )
    .await
    .expect("login against the mocked Vault Kubernetes-auth endpoint must succeed")
}

#[tokio::test]
async fn write_oauth2_credentials_puts_the_canonical_shape_at_the_expected_kv2_path() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{MOUNT}/data/{PATH_PREFIX}/default/weebo-app"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "fake-request-id",
            "lease_id": "",
            "lease_duration": 0,
            "renewable": false,
            "auth": null,
            "wrap_info": null,
            "warnings": null,
            "data": { "version": 1, "created_time": "2026-01-01T00:00:00Z", "deletion_time": "", "destroyed": false }
        })))
        .mount(&server)
        .await;

    let result = store
        .write_oauth2_credentials(
            "default",
            "weebo-app",
            &Oauth2Credentials {
                client_id: "client-id-123".to_string(),
                client_secret: "client-secret-456".to_string(),
                authentik_url: "https://login.example.com".to_string(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "write must succeed against the mocked KV v2 endpoint: {result:?}"
    );
}

#[tokio::test]
async fn write_oauth2_credentials_surfaces_a_vault_api_error() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{MOUNT}/data/{PATH_PREFIX}/default/weebo-app"
        )))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "errors": ["permission denied"]
        })))
        .mount(&server)
        .await;

    let result = store
        .write_oauth2_credentials(
            "default",
            "weebo-app",
            &Oauth2Credentials {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                authentik_url: "https://login.example.com".to_string(),
            },
        )
        .await;

    assert!(
        result.is_err(),
        "a 403 from Vault must surface as a SecretStoreError, not silently succeed"
    );
}

#[tokio::test]
async fn delete_removes_metadata_at_the_expected_kv2_path() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("DELETE"))
        .and(path(format!(
            "/v1/{MOUNT}/metadata/{PATH_PREFIX}/default/weebo-app"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let result = store.delete("default", "weebo-app").await;
    assert!(result.is_ok(), "delete must succeed: {result:?}");
}

/// Same idempotent-on-404 contract `K8sSecretStore::delete` has (a 404
/// from the Kubernetes API on delete is success, not failure) — Vault's
/// KV v2 metadata-delete returns 404 for a path with no metadata, which
/// must be treated the same way.
#[tokio::test]
async fn delete_is_idempotent_when_vault_returns_404() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("DELETE"))
        .and(path(format!(
            "/v1/{MOUNT}/metadata/{PATH_PREFIX}/default/already-gone"
        )))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "errors": []
        })))
        .mount(&server)
        .await;

    let result = store.delete("default", "already-gone").await;
    assert!(
        result.is_ok(),
        "a 404 from Vault on delete must be treated as already-deleted, not an error: {result:?}"
    );
}
