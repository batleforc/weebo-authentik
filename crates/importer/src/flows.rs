//! Shared flow pk -> slug resolution, used by brands and application
//! providers alike — the inverse of
//! `AuthentikHttpGateway::resolve_flow` (slug -> pk), needed because
//! every CRD spec that references a flow does so by slug, while
//! Authentik's API returns flow UUIDs.

use std::collections::HashMap;

use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::flows_api;
use uuid::Uuid;

pub async fn flow_pk_to_slug_map(
    configuration: &Configuration,
) -> anyhow::Result<HashMap<Uuid, String>> {
    let list = flows_api::flows_instances_list(
        configuration,
        None,       // denied_action
        None,       // designation
        None,       // flow_uuid
        None,       // name
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // search
        None,       // slug
    )
    .await?;

    Ok(list
        .results
        .into_iter()
        .map(|flow| (flow.pk, flow.slug))
        .collect())
}
