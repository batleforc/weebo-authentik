//! Imports Authentik groups into `AuthentikGroup` CRs. Returns a
//! pk -> name map, reused by users/policy-bindings to resolve group
//! references by name — the same convention
//! `AuthentikGateway::create_group`'s doc comment establishes for the
//! reconciler's own group resolution.

use std::collections::HashMap;
use std::path::Path;

use api::AuthentikGroup;
use api::group::AuthentikGroupSpec;
use api::status::AuthentikStatus;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::core_api;
use kube::api::ObjectMeta;
use uuid::Uuid;

pub async fn import_groups(
    configuration: &Configuration,
    out_dir: &Path,
) -> anyhow::Result<HashMap<Uuid, String>> {
    let list = core_api::core_groups_list(
        configuration,
        None,       // attributes
        None,       // include_children
        None,       // include_inherited_roles
        None,       // include_parents
        None,       // include_users
        None,       // is_superuser
        None,       // members_by_pk
        None,       // members_by_username
        None,       // name
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // search
    )
    .await?;

    let pk_to_name: HashMap<Uuid, String> = list
        .results
        .iter()
        .map(|g| (g.pk, g.name.clone()))
        .collect();

    for group in &list.results {
        // Authentik supports multi-parent groups; AuthentikGroupSpec's
        // `parent_ref` is a single `Option<String>` (a pre-existing CRD
        // schema constraint, not something this tool changes) — take
        // the first parent if there's more than one.
        let parent_ref = group
            .parents
            .as_ref()
            .and_then(|parents| parents.first())
            .and_then(|pk| pk_to_name.get(pk))
            .cloned();

        let attributes = group
            .attributes
            .as_ref()
            .map(|attrs| {
                attrs
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let name = crate::common::slugify(&group.name);
        let cr = AuthentikGroup {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikGroupSpec {
                name: group.name.clone(),
                is_superuser: group.is_superuser.unwrap_or(false),
                parent_ref,
                attributes,
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(group.pk.to_string()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikgroup", &name, &cr)?;
    }

    Ok(pk_to_name)
}
