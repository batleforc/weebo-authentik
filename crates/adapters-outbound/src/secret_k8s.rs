use std::collections::BTreeMap;

use application::ports::{Oauth2Credentials, SecretStore, SecretStoreError};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::{Api, ObjectMeta, Patch, PatchParams};

const FIELD_MANAGER: &str = "weebo-authentik-operator";

/// Fully implemented — a handful of straightforward Kubernetes Secret
/// calls, no reason to stub it. Writes the canonical shape used by every
/// oauth2 app in the Terraform module being replaced:
/// `AUTHENTIK_CLIENT_ID`/`AUTHENTIK_CLIENT_SECRET`/`AUTHENTIK_URL`. Never
/// called for proxy providers — see `.prompt/plan.md`.
pub struct K8sSecretStore {
    client: Client,
}

impl K8sSecretStore {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl SecretStore for K8sSecretStore {
    async fn write_oauth2_credentials(
        &self,
        namespace: &str,
        name: &str,
        credentials: &Oauth2Credentials,
    ) -> Result<(), SecretStoreError> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        let mut data = BTreeMap::new();
        data.insert(
            "AUTHENTIK_CLIENT_ID".to_string(),
            ByteString(credentials.client_id.as_bytes().to_vec()),
        );
        data.insert(
            "AUTHENTIK_CLIENT_SECRET".to_string(),
            ByteString(credentials.client_secret.as_bytes().to_vec()),
        );
        data.insert(
            "AUTHENTIK_URL".to_string(),
            ByteString(credentials.authentik_url.as_bytes().to_vec()),
        );

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        api.patch(
            name,
            &PatchParams::apply(FIELD_MANAGER).force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(|e| SecretStoreError::Write(e.to_string()))?;

        Ok(())
    }

    async fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        match api.delete(name, &Default::default()).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
            Err(e) => Err(SecretStoreError::Delete(e.to_string())),
        }
    }
}
