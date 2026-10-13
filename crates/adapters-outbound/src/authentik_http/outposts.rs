use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use api::AuthentikOutpost;
use api::outpost::OutpostType;
use application::ports::GatewayError;
use authentik_client::apis::outposts_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err, map_own_err, parse_i32};

/// Authentik's outpost embedded with every installation, never created by
/// this operator — the default target for a proxy provider whose
/// `outpostRef` is unset. See `.prompt/plan.md`.
const EMBEDDED_OUTPOST_NAME: &str = "authentik Embedded Outpost";

fn outpost_type(t: &OutpostType) -> models::OutpostTypeEnum {
    match t {
        OutpostType::Proxy => models::OutpostTypeEnum::Proxy,
        OutpostType::Ldap => models::OutpostTypeEnum::Ldap,
        OutpostType::Radius => models::OutpostTypeEnum::Radius,
    }
}

fn outpost_config(
    config: &serde_json::Value,
) -> std::collections::HashMap<String, serde_json::Value> {
    config
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect()
}

impl AuthentikHttpGateway {
    /// `outpost_ref` is `ProxyProviderSpec.outpost_ref` verbatim: `None`
    /// resolves to Authentik's embedded outpost, `Some` resolves an
    /// `AuthentikOutpost` by matching `name` (same caveat as
    /// `resolve_group_by_name` — no Kubernetes lookup here).
    async fn resolve_outpost(&self, outpost_ref: Option<&str>) -> Result<uuid::Uuid, GatewayError> {
        let name = outpost_ref.unwrap_or(EMBEDDED_OUTPOST_NAME);
        let list = outposts_api::outposts_instances_list(
            &self.configuration,
            None,
            None,
            None,
            Some(name),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(map_err)?;
        list.results
            .into_iter()
            .find(|o| o.name == name)
            .map(|o| o.pk)
            .ok_or_else(|| GatewayError::NotFound(format!("outpost {name:?} not found")))
    }

    pub(super) async fn create_outpost_impl(
        &self,
        outpost: &AuthentikOutpost,
    ) -> Result<String, GatewayError> {
        let req = models::OutpostRequest {
            name: outpost.spec.name.clone(),
            r#type: outpost_type(&outpost.spec.r#type),
            providers: vec![],
            service_connection: None,
            config: outpost_config(&outpost.spec.config),
            managed: None,
        };
        outposts_api::outposts_instances_create(&self.configuration, req)
            .await
            .map(|o| o.pk.to_string())
            .map_err(map_err)
    }

    pub(super) async fn update_outpost_impl(
        &self,
        authentik_id: &str,
        outpost: &AuthentikOutpost,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedOutpostRequest {
            name: Some(outpost.spec.name.clone()),
            r#type: Some(outpost_type(&outpost.spec.r#type)),
            providers: None,
            service_connection: None,
            config: Some(outpost_config(&outpost.spec.config)),
            managed: None,
        };
        outposts_api::outposts_instances_partial_update(
            &self.configuration,
            authentik_id,
            Some(req),
        )
        .await
        .map(|_| ())
        .map_err(map_own_err)
    }

    pub(super) async fn delete_outpost_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        ignore_not_found(
            outposts_api::outposts_instances_destroy(&self.configuration, authentik_id).await,
        )
    }

    /// Attaches the provider to the resolved outpost, and detaches it from
    /// every other outpost holding it (an `outpostRef` moved from A to B).
    /// Authentik has no "add one provider" call: each outpost's whole
    /// provider list is read, edited and written back, under a
    /// process-wide lock per outpost so that two applications attaching
    /// to the same outpost concurrently don't overwrite each other's
    /// edit. (Only this process' writers are serialized; the leader
    /// election keeps it the only one.)
    pub(super) async fn attach_outpost_impl(
        &self,
        outpost_ref: Option<&str>,
        provider_authentik_id: &str,
    ) -> Result<(), GatewayError> {
        let outpost_id = self.resolve_outpost(outpost_ref).await?.to_string();
        let provider_id = parse_i32(provider_authentik_id)?;

        let holding = outposts_api::outposts_instances_list(
            &self.configuration,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(1000),
            None,
            Some(vec![provider_id]),
            None,
            None,
            None,
        )
        .await
        .map_err(map_err)?;
        for other in holding.results {
            let other_id = other.pk.to_string();
            if other_id != outpost_id {
                self.edit_providers(&other_id, |providers| {
                    providers.retain(|p| *p != provider_id)
                })
                .await?;
            }
        }

        self.edit_providers(&outpost_id, |providers| {
            if !providers.contains(&provider_id) {
                providers.push(provider_id);
            }
        })
        .await
    }

    /// Read-modify-write of one outpost's provider list, under its lock;
    /// no write when `edit` changes nothing.
    async fn edit_providers(
        &self,
        outpost_id: &str,
        edit: impl FnOnce(&mut Vec<i32>),
    ) -> Result<(), GatewayError> {
        let lock = outpost_lock(&self.configuration.base_path, outpost_id);
        let _held = lock.lock().await;

        let outpost = outposts_api::outposts_instances_retrieve(&self.configuration, outpost_id)
            .await
            .map_err(map_err)?;
        let mut providers = outpost.providers.clone();
        edit(&mut providers);
        if providers == outpost.providers {
            return Ok(());
        }

        let patch = models::PatchedOutpostRequest {
            name: None,
            r#type: None,
            providers: Some(providers),
            service_connection: None,
            config: None,
            managed: None,
        };
        outposts_api::outposts_instances_partial_update(
            &self.configuration,
            outpost_id,
            Some(patch),
        )
        .await
        .map(|_| ())
        .map_err(map_err)
    }
}

/// One lock per outpost of an instance (`<api base>|<outpost pk>`), shared
/// by every gateway of the process — a gateway is built per reconcile.
fn outpost_lock(base_path: &str, outpost_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(Default::default);
    LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(format!("{base_path}|{outpost_id}"))
        .or_default()
        .clone()
}
