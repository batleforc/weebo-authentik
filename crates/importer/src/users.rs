//! Imports Authentik users into `AuthentikUser` CRs, resolving
//! `group_refs` from the already-imported groups' pk -> name map.

use std::collections::HashMap;
use std::path::Path;

use api::AuthentikUser;
use api::status::AuthentikStatus;
use api::user::AuthentikUserSpec;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::core_api;
use kube::api::ObjectMeta;
use uuid::Uuid;

pub async fn import_users(
    configuration: &Configuration,
    out_dir: &Path,
    groups_by_pk: &HashMap<Uuid, String>,
) -> anyhow::Result<()> {
    let list = core_api::core_users_list(
        configuration,
        None,       // attributes
        None,       // date_joined
        None,       // date_joined__gt
        None,       // date_joined__lt
        None,       // email
        None,       // groups_by_name
        None,       // groups_by_pk
        None,       // include_groups
        None,       // include_roles
        None,       // is_active
        None,       // is_superuser
        None,       // last_login
        None,       // last_login__gt
        None,       // last_login__isnull
        None,       // last_login__lt
        None,       // last_updated
        None,       // last_updated__gt
        None,       // last_updated__lt
        None,       // name
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // path
        None,       // path_startswith
        None,       // roles_by_name
        None,       // roles_by_pk
        None,       // search
        None,       // r#type
        None,       // username
        None,       // uuid
    )
    .await?;

    for user in &list.results {
        let group_refs = user
            .groups
            .as_ref()
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|pk| groups_by_pk.get(pk).cloned())
                    .collect()
            })
            .unwrap_or_default();

        let name = crate::slugify(&user.username);
        let cr = AuthentikUser {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikUserSpec {
                username: user.username.clone(),
                name: user.name.clone(),
                email: user.email.clone().unwrap_or_default(),
                is_active: user.is_active.unwrap_or(true),
                group_refs,
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(user.pk.to_string()),
                ..Default::default()
            }),
        };
        crate::write_cr(out_dir, "authentikuser", &name, &cr)?;
    }

    Ok(())
}
