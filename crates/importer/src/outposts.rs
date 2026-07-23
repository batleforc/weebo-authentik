//! Imports Authentik outposts into `AuthentikOutpost` CRs, skipping the
//! embedded default outpost (never created by this operator — see
//! `AuthentikHttpGateway::EMBEDDED_OUTPOST_NAME` in
//! `crates/adapters-outbound/src/authentik_http.rs`, same literal).
//! Returns the full raw list so `applications` can independently
//! resolve which outpost (if any) a proxy provider is attached to.

use std::path::Path;

use api::AuthentikOutpost;
use api::outpost::{AuthentikOutpostSpec, OutpostType};
use api::status::AuthentikStatus;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::outposts_api;
use authentik_client::models;
use kube::api::ObjectMeta;

/// Same literal as `AuthentikHttpGateway::EMBEDDED_OUTPOST_NAME` —
/// duplicated rather than shared across crates for one constant.
const EMBEDDED_OUTPOST_NAME: &str = "authentik Embedded Outpost";

pub async fn import_outposts(
    configuration: &Configuration,
    out_dir: &Path,
) -> anyhow::Result<Vec<models::Outpost>> {
    let list = outposts_api::outposts_instances_list(
        configuration,
        None,       // managed__icontains
        None,       // managed__iexact
        None,       // name__icontains
        None,       // name__iexact
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // providers__isnull
        None,       // providers_by_pk
        None,       // search
        None,       // service_connection__name__icontains
        None,       // service_connection__name__iexact
    )
    .await?;

    for outpost in &list.results {
        if outpost.name == EMBEDDED_OUTPOST_NAME {
            continue;
        }

        let kind = match outpost.r#type {
            models::OutpostTypeEnum::Proxy => OutpostType::Proxy,
            models::OutpostTypeEnum::Ldap => OutpostType::Ldap,
            models::OutpostTypeEnum::Radius => OutpostType::Radius,
            models::OutpostTypeEnum::Rac => {
                eprintln!(
                    "warning: outpost {:?} has type \"rac\", which AuthentikOutpostSpec's \
                     schema doesn't model (proxy/ldap/radius only) — skipping",
                    outpost.name
                );
                continue;
            }
        };

        let name = crate::common::slugify(&outpost.name);
        let cr = AuthentikOutpost {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikOutpostSpec {
                name: outpost.name.clone(),
                r#type: kind,
                config: serde_json::to_value(&outpost.config).unwrap_or_default(),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(outpost.pk.to_string()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikoutpost", &name, &cr)?;
    }

    Ok(list.results)
}
