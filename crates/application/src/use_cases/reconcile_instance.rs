use api::AuthentikInstance;
use domain::error::ReasonCode;
use weebo_kit_domain::vault_paths::{self, PathGrant};

use crate::use_cases::ReconcileOutcome;

/// `AuthentikInstance` has no corresponding Authentik-side object to
/// create — it just describes how to reach one. Only what the schema can't
/// fully check is validated here (`secretStore.vault.pathGrants`, which the
/// CRD normally refuses already). Actual connectivity/token validation
/// happens lazily, per call, in `GatewayFactory`
/// (`adapters_outbound::AuthentikGatewayFactory`).
pub fn reconcile_instance(instance: &AuthentikInstance) -> ReconcileOutcome {
    if let Some(vault) = &instance.spec.secret_store.vault {
        let grants: Vec<PathGrant<'_>> = vault
            .path_grants
            .iter()
            .map(|g| PathGrant {
                namespaces: &g.namespaces,
                cluster_scoped: g.cluster_scoped,
                paths: &g.paths,
            })
            .collect();
        if let Err(e) = vault_paths::validate(&grants) {
            return ReconcileOutcome::Errored {
                reason: ReasonCode::InvalidVaultPathGrants,
                message: format!("secretStore.vault.{e}"),
            };
        }
    }
    ReconcileOutcome::Synced { authentik_id: None }
}

#[cfg(test)]
mod tests {
    use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
    use kube::api::ObjectMeta;

    use super::*;

    #[test]
    fn accepted_cr_is_always_synced_with_no_remote_id() {
        let instance = AuthentikInstance {
            metadata: ObjectMeta {
                name: Some("prod".to_string()),
                ..Default::default()
            },
            spec: AuthentikInstanceSpec {
                url: "https://login.example.com".to_string(),
                token_secret_ref: SecretKeyRef {
                    name: "authentik-token".to_string(),
                    namespace: "authentik".to_string(),
                    key: "token".to_string(),
                },
                tls: Default::default(),
                secret_store: Default::default(),
            },
            status: None,
        };

        assert!(matches!(
            reconcile_instance(&instance),
            ReconcileOutcome::Synced { authentik_id: None }
        ));
    }

    #[test]
    fn invalid_path_grants_are_reported() {
        use api::instance::{SecretStoreSpec, VaultPathGrant, VaultSecretStoreSpec};

        let vault = |paths: &[&str]| VaultSecretStoreSpec {
            address: "https://vault:8200".to_string(),
            mount: "secret".to_string(),
            path_prefix: "weebo-authentik".to_string(),
            path_grants: vec![VaultPathGrant {
                namespaces: vec!["auth".to_string()],
                cluster_scoped: false,
                paths: paths.iter().map(|p| p.to_string()).collect(),
            }],
            kubernetes_auth_role: "auth".to_string(),
            kubernetes_auth_mount: "kubernetes".to_string(),
            ca_secret_ref: None,
        };
        let instance = |paths: &[&str]| AuthentikInstance {
            metadata: ObjectMeta {
                name: Some("prod".to_string()),
                ..Default::default()
            },
            spec: AuthentikInstanceSpec {
                url: "https://login.example.com".to_string(),
                token_secret_ref: SecretKeyRef {
                    name: "authentik-token".to_string(),
                    namespace: "authentik".to_string(),
                    key: "token".to_string(),
                },
                tls: Default::default(),
                secret_store: SecretStoreSpec {
                    vault: Some(vault(paths)),
                    ..Default::default()
                },
            },
            status: None,
        };

        assert!(matches!(
            reconcile_instance(&instance(&["batlehub/auth", "+/auth"])),
            ReconcileOutcome::Synced { .. }
        ));
        match reconcile_instance(&instance(&["+/*"])) {
            ReconcileOutcome::Errored { reason, message } => {
                assert_eq!(reason, ReasonCode::InvalidVaultPathGrants);
                assert!(
                    message.starts_with("secretStore.vault.pathGrants[0]"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
