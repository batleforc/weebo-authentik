//! Imports Authentik flows into `AuthentikFlow` CRs and returns a
//! pk -> slug map, reused by brands and application providers to resolve
//! flow references by slug — every CRD spec that references a flow does so
//! by slug, while Authentik's API returns flow UUIDs.

use std::collections::HashMap;
use std::path::Path;

use api::AuthentikFlow;
use api::flow::{
    AuthentikFlowSpec, FlowAuthentication, FlowDeniedAction, FlowDesignation, FlowLayout,
    PolicyEngineMode,
};
use api::status::AuthentikStatus;
use authentik_client::apis::configuration::Configuration;
use authentik_client::apis::flows_api;
use authentik_client::models;
use kube::api::ObjectMeta;
use uuid::Uuid;

/// Lists every flow, writes one `AuthentikFlow` CR per flow, and returns
/// the pk -> slug map the rest of the import needs. `status.authentikId`
/// is the slug (flows are slug-keyed, same as `AuthentikApplication`).
pub async fn import_flows(
    configuration: &Configuration,
    out_dir: &Path,
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

    let pk_to_slug: HashMap<Uuid, String> = list
        .results
        .iter()
        .map(|f| (f.pk, f.slug.clone()))
        .collect();

    for flow in &list.results {
        let name = crate::common::slugify(&flow.slug);
        let cr = AuthentikFlow {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                ..Default::default()
            },
            spec: AuthentikFlowSpec {
                slug: flow.slug.clone(),
                name: flow.name.clone(),
                title: flow.title.clone(),
                designation: map_designation(&flow.designation),
                authentication: flow.authentication.as_ref().map(map_authentication),
                policy_engine_mode: flow.policy_engine_mode.as_ref().map(map_policy_engine_mode),
                compatibility_mode: flow.compatibility_mode,
                layout: flow.layout.as_ref().map(map_layout),
                denied_action: flow.denied_action.as_ref().map(map_denied_action),
                background: flow.background.clone(),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some(flow.slug.clone()),
                ..Default::default()
            }),
        };
        crate::common::write_cr(out_dir, "authentikflow", &name, &cr)?;
    }

    Ok(pk_to_slug)
}

fn map_designation(d: &models::FlowDesignationEnum) -> FlowDesignation {
    use models::FlowDesignationEnum as M;
    match d {
        M::Authentication => FlowDesignation::Authentication,
        M::Authorization => FlowDesignation::Authorization,
        M::Invalidation => FlowDesignation::Invalidation,
        M::Enrollment => FlowDesignation::Enrollment,
        M::Unenrollment => FlowDesignation::Unenrollment,
        M::Recovery => FlowDesignation::Recovery,
        M::StageConfiguration => FlowDesignation::StageConfiguration,
    }
}

fn map_authentication(a: &models::AuthenticationEnum) -> FlowAuthentication {
    use models::AuthenticationEnum as M;
    match a {
        M::None => FlowAuthentication::None,
        M::RequireAuthenticated => FlowAuthentication::RequireAuthenticated,
        M::RequireUnauthenticated => FlowAuthentication::RequireUnauthenticated,
        M::RequireSuperuser => FlowAuthentication::RequireSuperuser,
        M::RequireRedirect => FlowAuthentication::RequireRedirect,
        M::RequireOutpost => FlowAuthentication::RequireOutpost,
        M::RequireToken => FlowAuthentication::RequireToken,
    }
}

fn map_policy_engine_mode(m: &models::PolicyEngineMode) -> PolicyEngineMode {
    use models::PolicyEngineMode as M;
    match m {
        M::All => PolicyEngineMode::All,
        M::Any => PolicyEngineMode::Any,
    }
}

fn map_layout(l: &models::FlowLayoutEnum) -> FlowLayout {
    use models::FlowLayoutEnum as M;
    match l {
        M::Stacked => FlowLayout::Stacked,
        M::ContentLeft => FlowLayout::ContentLeft,
        M::ContentRight => FlowLayout::ContentRight,
        M::SidebarLeft => FlowLayout::SidebarLeft,
        M::SidebarRight => FlowLayout::SidebarRight,
        M::SidebarLeftFrameBackground => FlowLayout::SidebarLeftFrameBackground,
        M::SidebarRightFrameBackground => FlowLayout::SidebarRightFrameBackground,
    }
}

fn map_denied_action(a: &models::DeniedActionEnum) -> FlowDeniedAction {
    use models::DeniedActionEnum as M;
    match a {
        M::MessageContinue => FlowDeniedAction::MessageContinue,
        M::Message => FlowDeniedAction::Message,
        M::Continue => FlowDeniedAction::Continue,
    }
}
