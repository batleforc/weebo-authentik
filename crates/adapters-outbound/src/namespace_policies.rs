//! [`KubePolicySource`]: the `AuthentikNamespacePolicy` rules, listed from
//! the API server on every call — for the admission webhook and the
//! reconcilers' re-check. Policies are few and rarely change; a fresh list
//! means a reconcile never acts on a policy that was just removed.

use api::AuthentikNamespacePolicy;
use api::namespace_policy::NamespaceRule;
use application::ports::{ClusterError, PolicySource};
use kube::{Api, Client};

pub struct KubePolicySource {
    client: Client,
}

impl KubePolicySource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl PolicySource<NamespaceRule> for KubePolicySource {
    async fn policies(&self) -> Result<Vec<Vec<NamespaceRule>>, ClusterError> {
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
