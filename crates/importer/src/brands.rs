//! Imports Authentik brands into `AuthentikBrand` CRs.

use std::collections::HashMap;
use std::path::Path;

use api::AuthentikBrand;
use api::brand::AuthentikBrandSpec;
use api::status::AuthentikStatus;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::core_api;
use kube::api::ObjectMeta;
use uuid::Uuid;

pub async fn import_brands(
    configuration: &Configuration,
    out_dir: &Path,
    flows_by_pk: &HashMap<Uuid, String>,
) -> anyhow::Result<()> {
    let list = core_api::core_brands_list(
        configuration,
        None,       // brand_uuid
        None,       // branding_default_flow_background
        None,       // branding_favicon
        None,       // branding_logo
        None,       // branding_title
        None,       // client_certificates
        None,       // default
        None,       // domain
        None,       // flow_authentication
        None,       // flow_device_code
        None,       // flow_invalidation
        None,       // flow_lockdown
        None,       // flow_recovery
        None,       // flow_unenrollment
        None,       // flow_user_settings
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // search
        None,       // web_certificate
    )
    .await?;

    for brand in &list.results {
        let resolve_flow = |flow: &Option<Option<Uuid>>| -> Option<String> {
            flow.as_ref()
                .and_then(|inner| inner.as_ref())
                .and_then(|pk| flows_by_pk.get(pk))
                .cloned()
        };

        let name = crate::common::slugify(&brand.domain);
        let cr = AuthentikBrand {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikBrandSpec {
                domain: brand.domain.clone(),
                default: brand.default.unwrap_or(false),
                branding_title: brand.branding_title.clone(),
                branding_logo: brand.branding_logo.clone(),
                branding_favicon: brand.branding_favicon.clone(),
                branding_default_flow_background: brand.branding_default_flow_background.clone(),
                // `default_application_ref` is deliberately never
                // populated: it names an AuthentikApplication CR, but
                // AuthentikBrand is cluster-scoped with no namespace to
                // search in — the same unresolved ambiguity
                // `AuthentikGateway::create_brand`'s doc comment flags.
                // A human fills this in by hand if needed.
                default_application_ref: None,
                flow_authentication: resolve_flow(&brand.flow_authentication),
                flow_invalidation: resolve_flow(&brand.flow_invalidation),
                flow_recovery: resolve_flow(&brand.flow_recovery),
                flow_unenrollment: resolve_flow(&brand.flow_unenrollment),
                flow_user_settings: resolve_flow(&brand.flow_user_settings),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(brand.brand_uuid.to_string()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikbrand", &name, &cr)?;
    }

    Ok(())
}
