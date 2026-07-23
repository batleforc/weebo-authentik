//! Imports Authentik applications + their oauth2/proxy providers into
//! `AuthentikApplication` CRs, and each application's policy bindings
//! into `AuthentikAccessPolicy` CRs.

use std::collections::HashMap;
use std::path::Path;

use api::access_policy::AuthentikAccessPolicySpec;
use api::application::{
    AuthentikApplicationSpec, MatchingMode, Oauth2ProviderSpec, ProviderKind, ProviderSpec,
    ProxyProviderSpec, RedirectUri,
};
use api::status::AuthentikStatus;
use api::{AuthentikAccessPolicy, AuthentikApplication};
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::{
    core_api, crypto_api, policies_api, propertymappings_api, providers_api,
};
use authentik_client::models;
use kube::api::ObjectMeta;
use uuid::Uuid;

const EMBEDDED_OUTPOST_NAME: &str = "authentik Embedded Outpost";

/// The lookup tables `import_applications` needs, pre-computed once in
/// `main.rs` by the other `import_*`/`flow_pk_to_slug_map` calls that run
/// before it.
pub(crate) struct ImportLookups<'a> {
    pub groups_by_pk: &'a HashMap<Uuid, String>,
    pub flows_by_pk: &'a HashMap<Uuid, String>,
    pub outposts: &'a [models::Outpost],
}

pub async fn import_applications(
    configuration: &Configuration,
    out_dir: &Path,
    instance_ref: &str,
    namespace: &str,
    application_slugs: Option<&[String]>,
    lookups: &ImportLookups<'_>,
) -> anyhow::Result<()> {
    let ImportLookups {
        groups_by_pk,
        flows_by_pk,
        outposts,
    } = *lookups;

    let property_mappings_by_pk = property_mapping_pk_to_name_map(configuration).await?;
    let certificates_by_pk = certificate_pk_to_name_map(configuration).await?;

    let providers = providers_api::providers_all_list(
        configuration,
        None,
        None,
        None,
        None,
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,
    )
    .await?;
    let providers_by_app_slug: HashMap<String, &models::Provider> = providers
        .results
        .iter()
        .filter_map(|p| p.assigned_application_slug.clone().map(|slug| (slug, p)))
        .collect();

    let apps = core_api::core_applications_list(
        configuration,
        None,       // for_user
        None,       // group
        None,       // meta_description
        None,       // meta_launch_url
        None,       // meta_publisher
        None,       // name
        None,       // only_with_launch_url
        None,       // ordering
        None,       // page
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,       // search
        None,       // slug
        None,       // superuser_full_list
    )
    .await?;

    for app in &apps.results {
        if let Some(allow) = application_slugs
            && !allow.iter().any(|s| s == &app.slug)
        {
            continue;
        }

        let Some(provider_summary) = providers_by_app_slug.get(&app.slug) else {
            eprintln!(
                "warning: application {:?} has no attached provider — \
                 AuthentikApplicationSpec.provider is required, skipping",
                app.slug
            );
            continue;
        };

        let provider_spec = match provider_summary.meta_model_name.as_str() {
            "authentik_providers_oauth2.oauth2provider" => {
                let full =
                    providers_api::providers_oauth2_retrieve(configuration, provider_summary.pk)
                        .await?;
                ProviderSpec {
                    kind: ProviderKind::Oauth2,
                    oauth2: Some(Oauth2ProviderSpec {
                        client_id: full.client_id.clone(),
                        authorization_flow: flows_by_pk
                            .get(&full.authorization_flow)
                            .cloned()
                            .unwrap_or_default(),
                        invalidation_flow: flows_by_pk
                            .get(&full.invalidation_flow)
                            .cloned()
                            .unwrap_or_default(),
                        signing_key: full
                            .signing_key
                            .as_ref()
                            .and_then(|inner| inner.as_ref())
                            .and_then(|pk| certificates_by_pk.get(pk))
                            .cloned(),
                        allowed_redirect_uris: full
                            .redirect_uris
                            .iter()
                            .map(|r| RedirectUri {
                                matching_mode: match r.matching_mode {
                                    models::MatchingModeEnum::Strict => MatchingMode::Strict,
                                    models::MatchingModeEnum::Regex => MatchingMode::Regex,
                                },
                                url: r.url.clone(),
                            })
                            .collect(),
                        property_mappings: full
                            .property_mappings
                            .as_ref()
                            .map(|pks| {
                                pks.iter()
                                    .filter_map(|pk| property_mappings_by_pk.get(pk).cloned())
                                    .collect()
                            })
                            .unwrap_or_default(),
                        grant_types: full
                            .grant_types
                            .as_ref()
                            .map(|gs| gs.iter().map(|g| g.to_string()).collect())
                            .unwrap_or_default(),
                    }),
                    proxy: None,
                }
            }
            "authentik_providers_proxy.proxyprovider" => {
                let full =
                    providers_api::providers_proxy_retrieve(configuration, provider_summary.pk)
                        .await?;
                let outpost_ref = outposts
                    .iter()
                    .find(|o| o.providers.contains(&full.pk))
                    .and_then(|o| {
                        if o.name == EMBEDDED_OUTPOST_NAME {
                            None
                        } else {
                            Some(crate::common::slugify(&o.name))
                        }
                    });
                ProviderSpec {
                    kind: ProviderKind::Proxy,
                    oauth2: None,
                    proxy: Some(ProxyProviderSpec {
                        internal_host: full.internal_host.clone().unwrap_or_default(),
                        external_host: full.external_host.clone(),
                        authorization_flow: flows_by_pk
                            .get(&full.authorization_flow)
                            .cloned()
                            .unwrap_or_default(),
                        invalidation_flow: flows_by_pk
                            .get(&full.invalidation_flow)
                            .cloned()
                            .unwrap_or_default(),
                        outpost_ref,
                    }),
                }
            }
            other => {
                eprintln!(
                    "warning: application {:?}'s provider has unsupported type {other:?} \
                     (only oauth2/proxy are modeled) — skipping",
                    app.slug
                );
                continue;
            }
        };

        let app_name = crate::common::slugify(&app.slug);
        let app_cr = AuthentikApplication {
            metadata: ObjectMeta {
                name: Some(app_name.clone()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: AuthentikApplicationSpec {
                instance_ref: instance_ref.to_string(),
                name: app.name.clone(),
                slug: app.slug.clone(),
                meta_icon: app.meta_icon.clone(),
                provider: provider_spec,
            },
            status: Some(AuthentikStatus {
                // The applications API is slug-keyed — see
                // `application::ports::AuthentikGateway::get_application`'s
                // doc comment, same convention here.
                authentik_id: Some(app.slug.clone()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikapplication", &app_name, &app_cr)?;

        import_policy_bindings(
            configuration,
            out_dir,
            namespace,
            &app_name,
            app.pk,
            groups_by_pk,
        )
        .await?;
    }

    Ok(())
}

async fn import_policy_bindings(
    configuration: &Configuration,
    out_dir: &Path,
    namespace: &str,
    app_name: &str,
    app_pk: Uuid,
    groups_by_pk: &HashMap<Uuid, String>,
) -> anyhow::Result<()> {
    let bindings = policies_api::policies_bindings_list(
        configuration,
        None,                      // enabled
        None,                      // order
        None,                      // ordering
        None,                      // page
        None,                      // page_size
        None,                      // policy
        None,                      // policy__isnull
        None,                      // search
        Some(&app_pk.to_string()), // target
        None,                      // target_in
        None,                      // timeout
    )
    .await?;

    for binding in &bindings.results {
        // Only group-targeted bindings map to AuthentikAccessPolicy —
        // user- or policy-targeted bindings have no equivalent in this
        // CRD's schema (mirrors `authentik_policy_binding`'s group-only
        // usage in the real Terraform module, see .prompt/plan.md).
        let Some(Some(group_pk)) = binding.group else {
            continue;
        };
        let Some(group_name) = groups_by_pk.get(&group_pk) else {
            continue;
        };

        let policy_name = format!("{app_name}-{}", crate::common::slugify(group_name));
        let policy_cr = AuthentikAccessPolicy {
            metadata: ObjectMeta {
                name: Some(policy_name.clone()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: AuthentikAccessPolicySpec {
                application_ref: app_name.to_string(),
                group_ref: group_name.clone(),
                order: binding.order,
                negate: binding.negate.unwrap_or(false),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(binding.pk.to_string()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikaccesspolicy", &policy_name, &policy_cr)?;
    }

    Ok(())
}

async fn property_mapping_pk_to_name_map(
    configuration: &Configuration,
) -> anyhow::Result<HashMap<Uuid, String>> {
    let list = propertymappings_api::propertymappings_all_list(
        configuration,
        None,
        None,
        None,
        None,
        None,
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,
    )
    .await?;
    Ok(list.results.into_iter().map(|m| (m.pk, m.name)).collect())
}

async fn certificate_pk_to_name_map(
    configuration: &Configuration,
) -> anyhow::Result<HashMap<Uuid, String>> {
    let list = crypto_api::crypto_certificatekeypairs_list(
        configuration,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(1000), // page_size — one-shot tool, no pagination; see main.rs
        None,
    )
    .await?;
    Ok(list.results.into_iter().map(|c| (c.pk, c.name)).collect())
}
