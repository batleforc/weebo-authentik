//! Small builder over `wiremock` for scripting Authentik API responses in
//! tests. Fully implemented — no reason to stub this, `wiremock` does the
//! work.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub struct AuthentikMock {
    pub server: MockServer,
}

impl AuthentikMock {
    pub async fn start() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_path(&self) -> String {
        self.server.uri()
    }

    /// Scripts a `GET /api/v3/core/applications/{slug}/` response.
    pub async fn mock_get_application(&self, slug: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v3/core/applications/{slug}/")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Scripts a `POST /api/v3/core/groups/` response (group create).
    pub async fn mock_create_group(&self, status: u16, body: serde_json::Value) {
        Mock::given(method("POST"))
            .and(path("/api/v3/core/groups/"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Scripts an arbitrary `GET <api_path>` response — the generic
    /// building block for scripting list/retrieve endpoints the
    /// `importer` crate's read-only tool needs (groups, users, brands,
    /// flows, providers, policy bindings, ...), rather than one
    /// hand-named method per endpoint.
    pub async fn mock_get(&self, api_path: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v3{api_path}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Scripts an arbitrary `POST <api_path>` response — the generic
    /// building block for scripting any create endpoint, including
    /// non-2xx responses (e.g. 400/409 conflicts), for adapter contract
    /// tests (`adapters-outbound`'s `AuthentikHttpGateway`).
    pub async fn mock_post(&self, api_path: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("POST"))
            .and(path(format!("/api/v3{api_path}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Scripts an arbitrary `PATCH <api_path>` response — the generic
    /// building block for scripting any partial-update endpoint.
    pub async fn mock_patch(&self, api_path: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v3{api_path}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Scripts an arbitrary `DELETE <api_path>` response — no body, since
    /// every delete endpoint this gateway calls returns `204 No Content`
    /// (or an error body on a non-2xx status).
    pub async fn mock_delete(&self, api_path: &str, status: u16) {
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v3{api_path}")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&self.server)
            .await;
    }
}
