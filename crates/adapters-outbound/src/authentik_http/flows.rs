use api::AuthentikFlow;
use api::flow::{
    FlowAuthentication, FlowDeniedAction, FlowDesignation, FlowLayout, PolicyEngineMode,
};
use application::ports::GatewayError;
use authentik_client::apis::flows_api;
use authentik_client::models;

use super::AuthentikHttpGateway;
use super::shared::{ignore_not_found, map_err};

impl AuthentikHttpGateway {
    pub(super) async fn create_flow_impl(
        &self,
        flow: &AuthentikFlow,
    ) -> Result<String, GatewayError> {
        let req = models::FlowRequest {
            name: flow.spec.name.clone(),
            slug: flow.spec.slug.clone(),
            title: flow.spec.title.clone(),
            designation: map_designation(&flow.spec.designation),
            background: flow.spec.background.clone(),
            policy_engine_mode: flow
                .spec
                .policy_engine_mode
                .as_ref()
                .map(map_policy_engine_mode),
            compatibility_mode: flow.spec.compatibility_mode,
            layout: flow.spec.layout.as_ref().map(map_layout),
            denied_action: flow.spec.denied_action.as_ref().map(map_denied_action),
            authentication: flow.spec.authentication.as_ref().map(map_authentication),
        };
        // Flows are slug-keyed everywhere else (update/delete), so the
        // stored identity is the slug, not the returned `pk` UUID — same
        // convention as `create_application`.
        flows_api::flows_instances_create(&self.configuration, req)
            .await
            .map(|f| f.slug)
            .map_err(map_err)
    }

    pub(super) async fn update_flow_impl(
        &self,
        authentik_id: &str,
        flow: &AuthentikFlow,
    ) -> Result<(), GatewayError> {
        let req = models::PatchedFlowRequest {
            name: Some(flow.spec.name.clone()),
            slug: Some(flow.spec.slug.clone()),
            title: Some(flow.spec.title.clone()),
            designation: Some(map_designation(&flow.spec.designation)),
            background: flow.spec.background.clone(),
            policy_engine_mode: flow
                .spec
                .policy_engine_mode
                .as_ref()
                .map(map_policy_engine_mode),
            compatibility_mode: flow.spec.compatibility_mode,
            layout: flow.spec.layout.as_ref().map(map_layout),
            denied_action: flow.spec.denied_action.as_ref().map(map_denied_action),
            authentication: flow.spec.authentication.as_ref().map(map_authentication),
        };
        flows_api::flows_instances_partial_update(&self.configuration, authentik_id, Some(req))
            .await
            .map(|_| ())
            .map_err(map_err)
    }

    pub(super) async fn delete_flow_impl(&self, authentik_id: &str) -> Result<(), GatewayError> {
        ignore_not_found(
            flows_api::flows_instances_destroy(&self.configuration, authentik_id).await,
        )
    }
}

// The `api` crate defines its own enums (it has no `authentik-client`
// dependency — hexagonal boundary), so the wire mapping lives here. Both
// sides carry identical variant sets; a `match` (not a `From`) keeps this
// exhaustive so adding a variant on either side fails to compile until
// mapped.
fn map_designation(d: &FlowDesignation) -> models::FlowDesignationEnum {
    use models::FlowDesignationEnum as M;
    match d {
        FlowDesignation::Authentication => M::Authentication,
        FlowDesignation::Authorization => M::Authorization,
        FlowDesignation::Invalidation => M::Invalidation,
        FlowDesignation::Enrollment => M::Enrollment,
        FlowDesignation::Unenrollment => M::Unenrollment,
        FlowDesignation::Recovery => M::Recovery,
        FlowDesignation::StageConfiguration => M::StageConfiguration,
    }
}

fn map_authentication(a: &FlowAuthentication) -> models::AuthenticationEnum {
    use models::AuthenticationEnum as M;
    match a {
        FlowAuthentication::None => M::None,
        FlowAuthentication::RequireAuthenticated => M::RequireAuthenticated,
        FlowAuthentication::RequireUnauthenticated => M::RequireUnauthenticated,
        FlowAuthentication::RequireSuperuser => M::RequireSuperuser,
        FlowAuthentication::RequireRedirect => M::RequireRedirect,
        FlowAuthentication::RequireOutpost => M::RequireOutpost,
        FlowAuthentication::RequireToken => M::RequireToken,
    }
}

fn map_policy_engine_mode(m: &PolicyEngineMode) -> models::PolicyEngineMode {
    use models::PolicyEngineMode as M;
    match m {
        PolicyEngineMode::All => M::All,
        PolicyEngineMode::Any => M::Any,
    }
}

fn map_layout(l: &FlowLayout) -> models::FlowLayoutEnum {
    use models::FlowLayoutEnum as M;
    match l {
        FlowLayout::Stacked => M::Stacked,
        FlowLayout::ContentLeft => M::ContentLeft,
        FlowLayout::ContentRight => M::ContentRight,
        FlowLayout::SidebarLeft => M::SidebarLeft,
        FlowLayout::SidebarRight => M::SidebarRight,
        FlowLayout::SidebarLeftFrameBackground => M::SidebarLeftFrameBackground,
        FlowLayout::SidebarRightFrameBackground => M::SidebarRightFrameBackground,
    }
}

fn map_denied_action(a: &FlowDeniedAction) -> models::DeniedActionEnum {
    use models::DeniedActionEnum as M;
    match a {
        FlowDeniedAction::MessageContinue => M::MessageContinue,
        FlowDeniedAction::Message => M::Message,
        FlowDeniedAction::Continue => M::Continue,
    }
}
