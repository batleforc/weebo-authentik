//! Test-only `AuthentikGateway` double, shared by every `reconcile_*` unit
//! test that needs to script a gateway response without doing real I/O —
//! layer 1 of `.prompt/plan.md`'s test strategy ("Unit domain" logic
//! tested without I/O at all). Kept separate from `testkit` (which wires
//! real `wiremock`/`envtest` I/O for layers 2-3): these tests must stay
//! synchronous-fast and dependency-free, not spin up a mock HTTP server.

use std::collections::VecDeque;
use std::sync::Mutex;

use api::application::{Oauth2ProviderSpec, ProviderKind, ProxyProviderSpec};
use api::{
    AuthentikApplication, AuthentikBrand, AuthentikFlow, AuthentikGroup, AuthentikOutpost,
    AuthentikScopeMapping, AuthentikUser,
};

use crate::ports::{AuthentikGateway, GatewayError, Oauth2ProviderUpsertResult, RemoteApplication};

/// Scriptable `AuthentikGateway` double. Every field defaults to `None`; a
/// method whose field is unset panics naming itself if called — a test
/// that exercises an unscripted call path should fail loudly at the exact
/// call site, not silently return a default. `create_result`/
/// `update_result` are reused across every CRD's create/update method
/// (group, user, outpost, brand, scope mapping): only one CRD's reconciler
/// is under test at a time, so a single pair of fields is enough.
#[derive(Default)]
pub struct FakeGateway {
    pub create_result: Option<Result<String, GatewayError>>,
    pub update_result: Option<Result<(), GatewayError>>,
    pub upsert_policy_binding_result: Option<Result<String, GatewayError>>,
    pub get_application_result: Option<Result<RemoteApplication, GatewayError>>,
    pub delete_provider_result: Option<Result<(), GatewayError>>,
    pub upsert_oauth2_provider_result: Option<Result<Oauth2ProviderUpsertResult, GatewayError>>,
    pub upsert_proxy_provider_result: Option<Result<String, GatewayError>>,
    pub attach_outpost_result: Option<Result<(), GatewayError>>,
    pub get_default_brand_result: Option<Result<Option<String>, GatewayError>>,
    /// Every `set_brand_default(id, default)` call, in order (always `Ok`).
    pub set_brand_default_calls: Mutex<Vec<(String, bool)>>,
    /// Every `delete_provider` call's provider id, in order.
    pub deleted_providers: Mutex<Vec<i32>>,
    /// Consumed first, in order, by `upsert_policy_binding` — for a test
    /// scripting a sequence (a `Gone` then the recreate) — before falling
    /// back to `upsert_policy_binding_result`.
    pub upsert_policy_binding_sequence: Mutex<VecDeque<Result<String, GatewayError>>>,
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
        self.take_create()
    }
    async fn update_application(
        &self,
        _authentik_id: &str,
        _app: &AuthentikApplication,
        _provider_id: i32,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_application(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_application not scripted on FakeGateway")
    }
    async fn get_application(
        &self,
        _authentik_id: &str,
    ) -> Result<RemoteApplication, GatewayError> {
        self.get_application_result
            .clone()
            .expect("test did not script a get_application response on FakeGateway")
    }
    async fn delete_provider(
        &self,
        authentik_id: i32,
        _kind: ProviderKind,
    ) -> Result<(), GatewayError> {
        self.deleted_providers.lock().unwrap().push(authentik_id);
        self.delete_provider_result
            .clone()
            .expect("test did not script a delete_provider response on FakeGateway")
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

    async fn create_flow(&self, _flow: &AuthentikFlow) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_flow(
        &self,
        _authentik_id: &str,
        _flow: &AuthentikFlow,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_flow(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_flow not scripted on FakeGateway")
    }

    async fn create_scope_mapping(
        &self,
        _mapping: &AuthentikScopeMapping,
    ) -> Result<String, GatewayError> {
        self.take_create()
    }
    async fn update_scope_mapping(
        &self,
        _authentik_id: &str,
        _mapping: &AuthentikScopeMapping,
    ) -> Result<(), GatewayError> {
        self.take_update()
    }
    async fn delete_scope_mapping(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_scope_mapping not scripted on FakeGateway")
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
        self.attach_outpost_result
            .clone()
            .expect("test did not script an attach_outpost response on FakeGateway")
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
        self.get_default_brand_result
            .clone()
            .expect("test did not script a get_default_brand response on FakeGateway")
    }
    async fn set_brand_default(
        &self,
        authentik_id: &str,
        default: bool,
    ) -> Result<(), GatewayError> {
        self.set_brand_default_calls
            .lock()
            .unwrap()
            .push((authentik_id.to_string(), default));
        Ok(())
    }

    async fn upsert_oauth2_provider(
        &self,
        _authentik_id: Option<&str>,
        _name: &str,
        _slug: &str,
        _spec: &Oauth2ProviderSpec,
    ) -> Result<Oauth2ProviderUpsertResult, GatewayError> {
        self.upsert_oauth2_provider_result
            .clone()
            .expect("test did not script an upsert_oauth2_provider response on FakeGateway")
    }
    async fn upsert_proxy_provider(
        &self,
        _authentik_id: Option<&str>,
        _name: &str,
        _spec: &ProxyProviderSpec,
    ) -> Result<String, GatewayError> {
        self.upsert_proxy_provider_result
            .clone()
            .expect("test did not script an upsert_proxy_provider response on FakeGateway")
    }

    async fn upsert_policy_binding(
        &self,
        _authentik_id: Option<&str>,
        _application_authentik_id: &str,
        _group_name: &str,
        _order: i32,
        _negate: bool,
    ) -> Result<String, GatewayError> {
        if let Some(next) = self
            .upsert_policy_binding_sequence
            .lock()
            .unwrap()
            .pop_front()
        {
            return next;
        }
        self.upsert_policy_binding_result
            .clone()
            .expect("test did not script an upsert_policy_binding response on FakeGateway")
    }
    async fn delete_policy_binding(&self, _authentik_id: &str) -> Result<(), GatewayError> {
        unimplemented!("delete_policy_binding not scripted on FakeGateway")
    }
}
