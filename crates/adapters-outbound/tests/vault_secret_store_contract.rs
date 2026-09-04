//! Layer-2 contract tests (`.prompt/plan.md`'s test strategy), mirroring
//! `authentik_http_contract.rs`'s pattern but for `VaultSecretStore`:
//! script Vault's HTTP KV v2 + Kubernetes-auth API via `wiremock` and
//! assert `VaultSecretStore` hits the right paths/bodies and maps
//! responses correctly (in particular, the idempotent-404-on-delete
//! behavior every `SecretStore` backend must share with
//! `K8sSecretStore::delete`). No real Vault instance involved.

use std::time::Duration;

use adapters_outbound::VaultSecretStore;
use api::instance::VaultSecretStoreSpec;
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
/// addition to (for a login) the nested `auth` object. `lease_duration`/
/// `renewable` on the nested `auth` object drive `VaultSecretStore`'s
/// cache TTL, so they are parameterized here.
fn login_response_with_lease(lease_duration: u64, renewable: bool) -> serde_json::Value {
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
            "lease_duration": lease_duration,
            "renewable": renewable,
            "entity_id": "fake-entity",
            "token_type": "service",
            "orphan": true
        }
    })
}

fn login_response() -> serde_json::Value {
    login_response_with_lease(3600, true)
}

fn write_path() -> String {
    format!("/v1/{MOUNT}/data/{PATH_PREFIX}/default/weebo-app")
}

fn credentials() -> Oauth2Credentials {
    Oauth2Credentials {
        client_id: "client-id-123".to_string(),
        client_secret: "client-secret-456".to_string(),
        authentik_url: "https://login.example.com".to_string(),
    }
}

fn spec(server: &MockServer) -> VaultSecretStoreSpec {
    VaultSecretStoreSpec {
        address: server.uri(),
        mount: MOUNT.to_string(),
        path_prefix: PATH_PREFIX.to_string(),
        kubernetes_auth_role: ROLE.to_string(),
        kubernetes_auth_mount: "kubernetes".to_string(),
        ca_secret_ref: None,
    }
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
    VaultSecretStore::new(&spec(server), FAKE_JWT, None)
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

/// KV v2 `set` always creates a new version, even for a byte-identical
/// document, and the application reconciler rewrites credentials on every
/// pass (requeued every 300s). Writing unconditionally therefore churned one
/// Vault version per application per five minutes forever. `write_path` reads
/// first and skips the `set` when nothing would change.
#[tokio::test]
async fn write_oauth2_credentials_skips_the_set_when_the_stored_document_already_matches() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    // Vault answers the read with exactly what we are about to write.
    Mock::given(method("GET"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "fake-request-id",
            "lease_id": "",
            "lease_duration": 0,
            "renewable": false,
            "auth": null,
            "wrap_info": null,
            "warnings": null,
            "data": {
                "data": {
                    "AUTHENTIK_CLIENT_ID": "client-id-123",
                    "AUTHENTIK_CLIENT_SECRET": "client-secret-456",
                    "AUTHENTIK_URL": "https://login.example.com"
                },
                "metadata": {
                    "created_time": "2026-01-01T00:00:00Z",
                    "deletion_time": "",
                    "destroyed": false,
                    "version": 7
                }
            }
        })))
        .mount(&server)
        .await;

    // ...and the write endpoint must never be called. Verified on drop.
    Mock::given(method("POST"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let result = store
        .write_oauth2_credentials("default", "weebo-app", &credentials())
        .await;

    assert!(
        result.is_ok(),
        "a no-op write must report success, not an error: {result:?}"
    );
}

/// The other half of the contract: skipping is conditional on the document
/// actually matching. A stored credential that has drifted -- here a rotated
/// client_secret -- must still be rewritten, or the skip would turn into a
/// permanent failure to converge.
#[tokio::test]
async fn write_oauth2_credentials_still_writes_when_the_stored_document_differs() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("GET"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "fake-request-id",
            "lease_id": "",
            "lease_duration": 0,
            "renewable": false,
            "auth": null,
            "wrap_info": null,
            "warnings": null,
            "data": {
                "data": {
                    "AUTHENTIK_CLIENT_ID": "client-id-123",
                    "AUTHENTIK_CLIENT_SECRET": "a-stale-secret",
                    "AUTHENTIK_URL": "https://login.example.com"
                },
                "metadata": {
                    "created_time": "2026-01-01T00:00:00Z",
                    "deletion_time": "",
                    "destroyed": false,
                    "version": 7
                }
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "fake-request-id",
            "lease_id": "",
            "lease_duration": 0,
            "renewable": false,
            "auth": null,
            "wrap_info": null,
            "warnings": null,
            "data": { "version": 8, "created_time": "2026-01-01T00:00:00Z", "deletion_time": "", "destroyed": false }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let result = store
        .write_oauth2_credentials("default", "weebo-app", &credentials())
        .await;

    assert!(
        result.is_ok(),
        "drifted credentials must be rewritten: {result:?}"
    );
}

/// A read that fails for any reason -- most importantly the 404 of a path
/// that does not exist yet -- must fall through to the write, never suppress
/// it. This is the first-write case, and it is the one where a wrong answer
/// would silently strand a consumer with no credentials at all.
#[tokio::test]
async fn write_oauth2_credentials_writes_when_the_path_does_not_exist_yet() {
    let server = MockServer::start().await;
    let store = store(&server).await;

    Mock::given(method("GET"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "errors": []
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(write_path()))
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
        .expect(1)
        .mount(&server)
        .await;

    let result = store
        .write_oauth2_credentials("default", "weebo-app", &credentials())
        .await;

    assert!(
        result.is_ok(),
        "a 404 from the pre-write read must not block the first write: {result:?}"
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

/// The cache TTL the factory reuses a store for is 80% of the login
/// lease — headroom to refresh before the Vault token actually expires.
#[tokio::test]
async fn cache_ttl_is_eighty_percent_of_a_renewable_lease() {
    let server = MockServer::start().await;
    let store = store(&server).await; // login_response(): lease 3600s, renewable

    assert_eq!(store.cache_ttl(), Some(Duration::from_secs(3600 * 4 / 5)));
}

/// A non-renewable (or zero-lease) login token must not be cached — the
/// factory re-logs-in every call rather than risk pinning a dead token.
#[tokio::test]
async fn cache_ttl_is_none_for_a_non_renewable_login() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(login_response_with_lease(3600, false)),
        )
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), FAKE_JWT, None)
        .await
        .expect("login must succeed");

    assert_eq!(store.cache_ttl(), None);
}

/// A 403 from Vault on a write means the token lapsed or was revoked out
/// of band — the store must re-login once and retry, not fail the write.
#[tokio::test]
async fn write_relogins_and_retries_once_on_a_403() {
    let server = MockServer::start().await;
    // Exactly two logins: the initial one plus the self-heal.
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(login_response()))
        .expect(2)
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), FAKE_JWT, None)
        .await
        .expect("login must succeed");

    // First write attempt is rejected as unauthorized (higher priority,
    // one-shot); the retry after re-login lands on the 200.
    Mock::given(method("POST"))
        .and(path(write_path()))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "errors": ["permission denied"]
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(write_path()))
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
        .write_oauth2_credentials("default", "weebo-app", &credentials())
        .await;

    assert!(
        result.is_ok(),
        "a 403 must trigger a re-login and a successful retry, not surface as an error: {result:?}"
    );
    // Verifies the login `.expect(2)` — the self-heal login actually fired.
    server.verify().await;
}

/// Same self-heal contract on delete: a 403 re-logs-in and retries, and
/// the retry still honors the idempotent-404 rule.
#[tokio::test]
async fn delete_relogins_and_retries_once_on_a_403() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(login_response()))
        .expect(2)
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), FAKE_JWT, None)
        .await
        .expect("login must succeed");

    let delete_path = format!("/v1/{MOUNT}/metadata/{PATH_PREFIX}/default/weebo-app");
    Mock::given(method("DELETE"))
        .and(path(delete_path.clone()))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "errors": ["permission denied"]
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(delete_path))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let result = store.delete("default", "weebo-app").await;

    assert!(
        result.is_ok(),
        "a 403 on delete must trigger a re-login and a successful retry: {result:?}"
    );
    server.verify().await;
}
