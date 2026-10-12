use std::sync::Arc;

use api::instance::{SecretKeyRef, TlsOptions};
use application::ports::{AuthentikGateway, BoundInstance, GatewayFactory, GatewayFactoryError};
use kube::{Client, ResourceExt};
use weebo_kit_adapters::Resolved;

use crate::authentik_http::AuthentikHttpGateway;
use crate::instance_resolver::AuthentikInstances;
use crate::secret_ref::read_secret_key;

/// Resolves an `AuthentikGateway` for a resolved instance — an
/// `AuthentikInstance` or an `AuthentikLocalInstance`.
///
/// The instance itself is read through the kit's `Instances` — served from
/// live reflector `Store`s when `operator::main` wires them in, or a live
/// apiserver call otherwise. The `tokenSecretRef` `Secret` is still read
/// fresh on every call so token rotation is picked up immediately, and the
/// built HTTP gateway is not cached — which is also what makes an edited
/// `spec.tls` (or a rotated CA in its `caSecretRef` Secret) take effect on
/// the next reconcile rather than on the next operator restart. A local
/// instance's token and CA live in its own namespace.
pub struct AuthentikGatewayFactory {
    client: Client,
    instances: Arc<AuthentikInstances>,
}

/// What a gateway is built from, whichever the instance kind.
struct Connection {
    url: String,
    token_secret_ref: SecretKeyRef,
    tls: TlsOptions,
}

impl AuthentikGatewayFactory {
    pub fn new(client: Client, instances: Arc<AuthentikInstances>) -> Self {
        Self { client, instances }
    }

    async fn build_gateway(
        &self,
        instance: Connection,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let secret_ref = &instance.token_secret_ref;
        let token_bytes = read_secret_key(&self.client, secret_ref, "secret")
            .await
            .map_err(GatewayFactoryError::ResolutionFailed)?;
        let token = String::from_utf8(token_bytes).map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!(
                "secret {}/{} key {:?} is not valid utf-8: {e}",
                secret_ref.namespace, secret_ref.name, secret_ref.key
            ))
        })?;

        // A private CA (e.g. the cluster's own PKI signing the Authentik
        // ingress) is read from its Secret via the API and handed to the HTTP
        // client — no mounted volume needed, which is what makes this usable
        // from a chart that exposes no `extraVolumes`.
        let ca_pem = self.read_tls_ca(&instance.tls).await?;
        let http_client = build_http_client(&instance.tls, ca_pem.as_deref())?;

        // `spec.url` is the WEB base; the REST base path is derived from it.
        // Handing the same string to both is what used to put `/api/v3`
        // inside every oauth2 app's OIDC issuer.
        let (api_base_path, web_base_url) = api::instance::split_urls(&instance.url);
        Ok(Arc::new(AuthentikHttpGateway::with_client(
            api_base_path,
            web_base_url,
            token,
            http_client,
        )))
    }

    /// Reads the optional `spec.tls.caSecretRef` PEM bundle from its
    /// Kubernetes `Secret`, returning `None` when no CA is configured. A
    /// configured-but-unreadable CA (missing Secret/key) is a hard error
    /// rather than a silent fall-back to the platform trust store —
    /// verifying a private-CA Authentik against the wrong roots is a
    /// security regression, not a warning. Mirrors
    /// `AuthentikSecretStoreFactory::read_vault_ca`.
    async fn read_tls_ca(&self, tls: &TlsOptions) -> Result<Option<Vec<u8>>, GatewayFactoryError> {
        let Some(ca_ref) = tls.ca_secret_ref.as_ref() else {
            return Ok(None);
        };
        read_secret_key(&self.client, ca_ref, "Authentik CA secret")
            .await
            .map(Some)
            .map_err(GatewayFactoryError::ResolutionFailed)
    }
}

/// The HTTP client the gateway talks to Authentik with, carrying the
/// instance's `spec.tls`. With neither option set this is the same client
/// the generated `Configuration` would have built for itself.
///
/// `ca_pem` is *added to* the platform trust store rather than replacing it
/// (`rustls-platform-verifier`'s extra-roots composition), so an instance
/// behind a private CA and one behind a public one can be served by the same
/// operator, and `SSL_CERT_FILE`/`SSL_CERT_DIR` keep working alongside it.
fn build_http_client(
    tls: &TlsOptions,
    ca_pem: Option<&[u8]>,
) -> Result<reqwest::Client, GatewayFactoryError> {
    let mut builder = reqwest::Client::builder();
    if let Some(pem) = ca_pem {
        // `from_pem_bundle` handles both a single cert and a concatenated
        // chain; each becomes an additional trusted root. Same handling as
        // the importer's `--ca-cert`.
        let certs = reqwest::Certificate::from_pem_bundle(pem).map_err(|e| {
            GatewayFactoryError::ResolutionFailed(format!(
                "parsing the Authentik CA PEM bundle from tls.caSecretRef: {e}"
            ))
        })?;
        // Input carrying no PEM block at all parses as an *empty* bundle
        // rather than an error (DER bytes in a `ca.crt`, a stray comment
        // file, a key instead of a cert). Left alone, that is precisely the
        // silent fall-back to the platform roots this field exists to avoid,
        // so an empty bundle is rejected explicitly.
        if certs.is_empty() {
            return Err(GatewayFactoryError::ResolutionFailed(
                "the CA bundle from tls.caSecretRef contains no PEM certificate".to_string(),
            ));
        }
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    if tls.insecure_skip_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder.build().map_err(|e| {
        GatewayFactoryError::ResolutionFailed(format!("building the Authentik HTTP client: {e}"))
    })
}

fn cluster_connection(instance: &api::AuthentikInstance) -> Connection {
    Connection {
        url: instance.spec.url.clone(),
        token_secret_ref: instance.spec.token_secret_ref.clone(),
        tls: instance.spec.tls.clone(),
    }
}

#[async_trait::async_trait]
impl GatewayFactory for AuthentikGatewayFactory {
    async fn gateway_for(
        &self,
        bound: &BoundInstance,
    ) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let connection = match self
            .instances
            .get(bound)
            .await
            .map_err(GatewayFactoryError::ResolutionFailed)?
            .ok_or_else(|| GatewayFactoryError::InstanceNotFound(bound.to_string()))?
        {
            Resolved::Cluster(instance) => cluster_connection(&instance),
            Resolved::Local(local) => {
                let namespace = local.namespace().unwrap_or_default();
                Connection {
                    url: local.spec.url.clone(),
                    token_secret_ref: local.spec.token_secret_ref.in_namespace(&namespace),
                    tls: local.spec.tls.in_namespace(&namespace),
                }
            }
        };
        self.build_gateway(connection).await
    }

    async fn default_gateway(&self) -> Result<Arc<dyn AuthentikGateway>, GatewayFactoryError> {
        let instances = self
            .instances
            .cluster_instances()
            .await
            .map_err(GatewayFactoryError::ResolutionFailed)?;

        // The kit's selection; no instance at all is ambiguous too here
        // (`AmbiguousDefaultInstance`), as it always was.
        let instance = weebo_kit_adapters::select(&instances, None)
            .map_err(|e| GatewayFactoryError::AmbiguousDefault(e.to_string()))?;
        self.build_gateway(cluster_connection(instance)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::instance::SecretKeyRef;

    /// A throwaway self-signed P-256 CA (CN=`weebo-authentik test CA`,
    /// valid until 2126), here only so `add_root_certificate` gets real DER
    /// to chew on — it signs nothing and is not trusted anywhere.
    const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIBmjCCAUGgAwIBAgIUDseMGg7lqwp/fehLDBBF+pqjkQowCgYIKoZIzj0EAwIw
IjEgMB4GA1UEAwwXd2VlYm8tYXV0aGVudGlrIHRlc3QgQ0EwIBcNMjYwOTAxMjIx
NDU5WhgPMjEyNjA4MDgyMjE0NTlaMCIxIDAeBgNVBAMMF3dlZWJvLWF1dGhlbnRp
ayB0ZXN0IENBMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEk/e90DXUvJu6vjN+
IkxWBZCEZoshuQYr9hRX+ppeJ/IxvMJM7jZObmxsLbN0H3JPdJCE8y8+g0wDDApa
Fh4O/aNTMFEwHQYDVR0OBBYEFHClE9JN2ravRskzWDnUoX+ACo1vMB8GA1UdIwQY
MBaAFHClE9JN2ravRskzWDnUoX+ACo1vMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZI
zj0EAwIDRwAwRAIgUHYMhR7mBxjF2RLdcroOL/tRDyGS8PcSgn/Ae1YeCdACIG7X
8ZJWYN/smaj63o+6SyWrQgzrQhnpf9NCD5aT7D+K
-----END CERTIFICATE-----
";

    #[test]
    fn no_tls_options_builds_the_plain_client() {
        build_http_client(&TlsOptions::default(), None).expect("default client must build");
    }

    #[test]
    fn a_pem_bundle_becomes_an_extra_root() {
        build_http_client(&TlsOptions::default(), Some(TEST_CA_PEM))
            .expect("a valid PEM bundle must build a client");
        // A concatenated chain is the other accepted shape (`from_pem_bundle`
        // yields one root per entry), and is what a `ca.crt` holding an
        // intermediate + root looks like.
        let chain = [TEST_CA_PEM, TEST_CA_PEM].concat();
        build_http_client(&TlsOptions::default(), Some(&chain))
            .expect("a concatenated PEM chain must build a client");
    }

    #[test]
    fn a_garbage_ca_is_an_error_not_a_silent_fallback() {
        // The whole point of erroring here: falling back to the platform
        // roots would mean verifying a private-CA instance against roots
        // that cannot vouch for it.
        let err = build_http_client(&TlsOptions::default(), Some(b"not a certificate"))
            .expect_err("garbage must not build a client");
        let message = err.to_string();
        assert!(
            message.contains("tls.caSecretRef"),
            "error should name the field at fault, got: {message}"
        );
    }

    #[test]
    fn insecure_skip_verify_still_builds() {
        let tls = TlsOptions {
            insecure_skip_verify: true,
            ca_secret_ref: None,
        };
        build_http_client(&tls, None).expect("the insecure escape hatch must build");
    }

    /// The two options are independent: a CA root and `insecureSkipVerify`
    /// may be set together (the skip wins at verification time), and that
    /// combination must not fail to build.
    #[test]
    fn a_ca_and_skip_verify_coexist() {
        let tls = TlsOptions {
            insecure_skip_verify: true,
            ca_secret_ref: Some(SecretKeyRef {
                name: "authentik-ca".to_string(),
                namespace: "weebo-authentik".to_string(),
                key: "ca.crt".to_string(),
            }),
        };
        build_http_client(&tls, Some(TEST_CA_PEM)).expect("both options at once must build");
    }
}
