//! [`KubePolicySource`]: the `AuthentikNamespacePolicy` rules, for the
//! admission webhook and the reconcilers' re-check. Read from the binary's
//! live store of every policy ([`KubePolicySource::with_store`], watched
//! without the label selector), from the API otherwise (tests). A store
//! that hasn't listed is never trusted empty: the read waits for it, then
//! falls back to the API.

use api::AuthentikNamespacePolicy;
use api::namespace_policy::NamespaceRule;
use application::ports::{ClusterError, PolicySource};
use kube::runtime::reflector::Store;
use kube::{Api, Client};
use weebo_kit_adapters::instance::STORE_READY_TIMEOUT;
use weebo_kit_adapters::wait_synced;

pub struct KubePolicySource {
    client: Client,
    store: Option<Store<AuthentikNamespacePolicy>>,
}

impl KubePolicySource {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: None,
        }
    }

    /// Reads the policies from `store` (every policy, whatever its labels:
    /// one another release selects still denies) once it has listed.
    pub fn with_store(mut self, store: Store<AuthentikNamespacePolicy>) -> Self {
        self.store = Some(store);
        self
    }
}

#[async_trait::async_trait]
impl PolicySource<NamespaceRule> for KubePolicySource {
    async fn policies(&self) -> Result<Vec<Vec<NamespaceRule>>, ClusterError> {
        if let Some(store) = &self.store
            && wait_synced(store, STORE_READY_TIMEOUT).await
        {
            return Ok(store.state().iter().map(|p| p.spec.rules.clone()).collect());
        }
        let api: Api<AuthentikNamespacePolicy> = Api::all(self.client.clone());
        Ok(api
            .list(&Default::default())
            .await
            .map_err(|e| ClusterError(format!("listing AuthentikNamespacePolicy: {e}")))?
            .into_iter()
            .map(|p| p.spec.rules)
            .collect())
    }
}
