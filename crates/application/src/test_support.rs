//! Test-only `AuthentikGateway` double, shared by every `reconcile_*` unit
//! test that needs to script a gateway response without doing real I/O —
//! layer 1 of `.prompt/plan.md`'s test strategy ("Unit domain" logic
//! tested without I/O at all). Kept separate from `testkit` (which wires
//! real `wiremock`/`envtest` I/O for layers 2-3): these tests must stay
//! synchronous-fast and dependency-free, not spin up a mock HTTP server.

use api::application::{Oauth2ProviderSpec, ProxyProviderSpec};
use api::{AuthentikApplication, AuthentikBrand, AuthentikGroup, AuthentikOutpost, AuthentikUser};

use crate::ports::{AuthentikGateway, GatewayError, Oauth2ProviderUpsertResult};

/// Scriptable `AuthentikGateway` double. Every field defaults to `None`; a
/// method whose field is unset panics naming itself if called — a test
/// that exercises an unscripted call path should fail loudly at the exact
/// call site, not silently return a default. `create_result`/
/// `update_result` are reused across every CRD's create/update method
/// (group, user, outpost, brand): only one CRD's reconciler is under test
/// at a time, so a single pair of fields is enough.
#[derive(Default)]
pub struct FakeGateway {
    pub create_result: Option<Result<String, GatewayError>>,
    pub update_result: Option<Result<(), GatewayError>>,
    pub upsert_policy_binding_result: Option<Result<String, GatewayError>>,
}

impl FakeGateway {
    pub fn create(result: Result<String, GatewayError>) -> Self {
        Self {
            create_result: Some(result),
            ..Default::default()
        }
    }

    pub fn update(result: Result<(), GatewayError>) -> Self {
        Self {
            update_result: Some(result),
            ..Default::default()
        }
    }

    pub fn upsert_policy_binding(result: Result<String, GatewayError>) -> Self {
        Self {
            upsert_policy_binding_result: Some(result),
            ..Default::default()
        }
    }

    fn take_create(&self) -> Result<String, GatewayError> {
        self.create_result
            .clone()
            .expect("test did not script a create_* response on FakeGateway")
    }

    fn take_update(&self) -> Result<(), GatewayError> {
        self.update_result
            .clone()
            .expect("test did not script an update_* response on FakeGateway")
    }
}

#[async_trait::async_trait]
impl AuthentikGateway for FakeGateway {
    async fn create_application(
        &self,
        _app: &AuthentikApplication,
        _provider_id: i32,
    ) -> Result<String, GatewayError> {
        unimplemented!("create_application not scripted on FakeGateway")
    }
    async fn update_application(
        &self,
        _authentik_id: &str,
        _app: &AuthentikApplication,
        _provider_id: i32,
    ) -> Result<(), GatewayError> {
        unimplemented!("update_application not scripted on FakeGateway")
    }
    async fn delete_application(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_application not scripted on FakeGateway")
    }
    async fn get_application(
        &self,
        _authentik_id: &str,
    ) -> Result<serde_json::Value, GatewayError> {
        unimplemented!("get_application not scripted on FakeGateway")
    }

    async fn create_group(&self, _group: &AuthentikGroup) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_group(
        &self,
        _authentik_id: &str,
        _group: &AuthentikGroup,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_group(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_group not scripted on FakeGateway")
    }

    async fn create_user(&self, _user: &AuthentikUser) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_user(
        &self,
        _authentik_id: &str,
        _user: &AuthentikUser,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_user(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_user not scripted on FakeGateway")
    }

    async fn create_outpost(&self, _outpost: &AuthentikOutpost) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_outpost(
        &self,
        _authentik_id: &str,
        _outpost: &AuthentikOutpost,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_outpost(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_outpost not scripted on FakeGateway")
    }
    async fn attach_outpost(
        &self,
        _outpost_ref: Option<&str>,
        _provider_authentik_id: &str,
    ) -> Result<(), GatewayError> {
        unimplemented!("attach_outpost not scripted on FakeGateway")
    }

    async fn create_brand(&self, _brand: &AuthentikBrand) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_brand(
        &self,
        _authentik_id: &str,
        _brand: &AuthentikBrand,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_brand(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_brand not scripted on FakeGateway")
    }
    async fn get_default_brand(&self) -> Result<Option<String>, GatewayError> {
        unimplemented!("get_default_brand not scripted on FakeGateway")
    }
    async fn set_brand_default(
        &self,
        _authentik_id: &str,
        _default: bool,
    ) -> Result<(), GatewayError> {
        unimplemented!("set_brand_default not scripted on FakeGateway")
    }

    async fn upsert_oauth2_provider(
        &self,
        _authentik_id: Option<&str>,
        _name: &str,
        _spec: &Oauth2ProviderSpec,
    ) -> Result<Oauth2ProviderUpsertResult, GatewayError> {
        unimplemented!("upsert_oauth2_provider not scripted on FakeGateway")
    }
    async fn upsert_proxy_provider(
        &self,
        _authentik_id: Option<&str>,
        _name: &str,
        _spec: &ProxyProviderSpec,
    ) -> Result<String, GatewayError> {
        unimplemented!("upsert_proxy_provider not scripted on FakeGateway")
    }

    async fn upsert_policy_binding(
        &self,
        _authentik_id: Option<&str>,
        _application_authentik_id: &str,
        _group_name: &str,
        _order: i32,
        _negate: bool,
    ) -> Result<String, GatewayError> {
        self.upsert_policy_binding_result
            .clone()
            .expect("test did not script an upsert_policy_binding response on FakeGateway")
    }
    async fn delete_policy_binding(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_policy_binding not scripted on FakeGateway")
    }
}
