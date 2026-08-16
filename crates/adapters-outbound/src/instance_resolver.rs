//! Resolves `AuthentikInstance` CRs for the gateway/secret-store
//! factories. Both factories need the same lookup, so it lives here once.
//!
//! When an in-memory reflector `Store` is wired in (via
//! `with_store`, done by `operator::main`), lookups are served from that
//! live-updated watch cache — no per-reconcile apiserver GET/LIST, and
//! config edits are reflected as soon as the watch delivers them. The
//! plain `new` constructor (used by tests that build a factory with just a
//! `kube::Client`) has no store and falls back to a live API call, exactly
//! as the factories did before caching existed. The store is also bypassed
//! if its writer has been dropped (operator shutdown).

use api::AuthentikInstance;
use kube::Client;
use kube::api::Api;
use kube::runtime::reflector::Store;

pub(crate) struct InstanceResolver {
    client: Client,
    store: Option<Store<AuthentikInstance>>,
}

impl InstanceResolver {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            store: None,
        }
    }

    pub(crate) fn with_store(client: Client, store: Store<AuthentikInstance>) -> Self {
        Self {
            client,
            store: Some(store),
        }
    }

    /// The single named instance. `Ok(None)` means it genuinely does not
    /// exist; `Err` is a lookup failure. The `String` is a
    /// caller-facing message each factory wraps in its own error type.
    pub(crate) async fn get(&self, name: &str) -> Result<Option<AuthentikInstance>, String> {
        // When a store is wired and ready, it is authoritative. If the
        // writer has been dropped (shutdown), `wait_until_ready` errors and
        // we fall through to a live lookup.
        if let Some(store) = &self.store
            && store.wait_until_ready().await.is_ok()
        {
            return Ok(store
                .find(|i| i.metadata.name.as_deref() == Some(name))
                .map(|arc| (*arc).clone()));
        }

        let api: Api<AuthentikInstance> = Api::all(self.client.clone());
        api.get_opt(name)
            .await
            .map_err(|e| format!("fetching AuthentikInstance {name:?}: {e}"))
    }

    /// Every instance in the cluster — used to elect the single default
    /// instance for CRDs that carry no explicit `instanceRef`.
    pub(crate) async fn list(&self) -> Result<Vec<AuthentikInstance>, String> {
        if let Some(store) = &self.store
            && store.wait_until_ready().await.is_ok()
        {
            return Ok(store.state().iter().map(|arc| (**arc).clone()).collect());
        }

        let api: Api<AuthentikInstance> = Api::all(self.client.clone());
        Ok(api
            .list(&Default::default())
            .await
            .map_err(|e| format!("listing AuthentikInstance: {e}"))?
            .items)
    }
}
