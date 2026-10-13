//! Imports the scope mappings made by hand (not Authentik's built-in ones)
//! into `AuthentikScopeMapping` CRs. The built-ins (`managed` set) stay
//! out: Authentik's own migrations own them and providers reference them
//! by name — a CR would delete one along with itself.

use std::path::Path;

use api::AuthentikScopeMapping;
use api::scope_mapping::AuthentikScopeMappingSpec;
use api::status::AuthentikStatus;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::propertymappings_api;
use kube::api::ObjectMeta;

/// Writes one `AuthentikScopeMapping` CR per unmanaged scope mapping, its
/// `status.authentikId` the mapping's pk.
pub async fn import_scope_mappings(
    configuration: &Configuration,
    out_dir: &Path,
) -> anyhow::Result<()> {
    let list = propertymappings_api::propertymappings_provider_scope_list(
        configuration,
        None,       // managed
        Some(true), // managed__isnull — hand-made mappings only
        None,       // name
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // scope_name
        None,       // search
    )
    .await?;

    for mapping in list
        .results
        .iter()
        .filter(|m| m.managed.as_ref().and_then(Option::as_ref).is_none())
    {
        let name = crate::common::slugify(&mapping.name);
        let cr = AuthentikScopeMapping {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikScopeMappingSpec {
                name: mapping.name.clone(),
                scope_name: mapping.scope_name.clone(),
                expression: mapping.expression.clone(),
                description: mapping.description.clone().filter(|d| !d.is_empty()),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(mapping.pk.to_string()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikscopemapping", &name, &cr)?;
    }
    Ok(())
}
