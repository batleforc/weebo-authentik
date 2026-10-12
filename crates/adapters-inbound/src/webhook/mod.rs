//! Admission webhook: adapts an incoming `AdmissionReview` into a call to
//! `application::use_cases::evaluate_admission::enforce` (the kit's
//! allow-list engine, through `domain::allow_list`) — the same check the
//! reconcilers repeat. A CR bound to an `AuthentikLocalInstance` isn't
//! governed by the policy (its namespace's own token bounds it). Plus
//! `/scope`, the kit's `ScopeGuard`.

use std::sync::Arc;

use api::namespace_policy::NamespaceRule;
use api::{AuthentikAccessPolicy, AuthentikApplication};
use application::ports::{BoundInstance, InstanceDirectory, PolicySource};
use application::use_cases::evaluate_admission::enforce;
use axum::Router;
use domain::allow_list::ResourceKind;
use domain::error::ReasonCode;
use kube::Client;
use kube::api::Api;
use kube::core::DynamicObject;
use weebo_kit_runtime::admission::{Admission, Denial, ScopeGuard, scope_router, typed};
use weebo_kit_runtime::{Health, WatchScope};

use crate::controller::FINALIZER;

#[derive(Clone)]
pub struct WebhookState {
    /// Every `AuthentikNamespacePolicy`, read on each request.
    pub policies: Arc<dyn PolicySource<NamespaceRule>>,
    /// Resolves the instance of the reviewed CR; `None`: every CR is
    /// governed by the policy.
    pub instances: Option<InstanceLookup>,
}

/// What the webhook needs to find a CR's instance.
#[derive(Clone)]
pub struct InstanceLookup {
    pub client: Client,
    pub directory: Arc<dyn InstanceDirectory>,
}

/// `POST /validate` (the kit's: a `DELETE`, or an `UPDATE` leaving `spec`
/// alone — the operator removing its finalizer — always passes), `POST
/// /scope` (no CR leaves `scope`'s label selector by accident), plus the
/// probes. Decisions are counted as `weebo_authentik_webhook_decisions_total`.
pub fn router(state: WebhookState, health: Arc<Health>, scope: WatchScope) -> Router {
    let guard = ScopeGuard {
        scope,
        finalizer: FINALIZER,
        annotations: api::ANNOTATIONS,
        reason: ReasonCode::LeavesWatchScope,
    };
    weebo_kit_runtime::admission::router(
        Arc::new(state),
        health,
        "weebo-authentik-operator",
        "weebo_authentik",
    )
    .merge(scope_router(Arc::new(guard)))
}

impl WebhookState {
    /// The reviewed CR is bound to an `AuthentikLocalInstance`. Anything
    /// that can't be resolved counts as governed: the policy decides.
    async fn bound_locally(
        &self,
        kind: ResourceKind,
        namespace: &str,
        object: &DynamicObject,
    ) -> bool {
        let Some(lookup) = &self.instances else {
            return false;
        };
        let application = match kind {
            ResourceKind::AuthentikApplication => typed::<AuthentikApplication>(object).ok(),
            ResourceKind::AuthentikAccessPolicy => {
                let Ok(policy) = typed::<AuthentikAccessPolicy>(object) else {
                    return false;
                };
                Api::<AuthentikApplication>::namespaced(lookup.client.clone(), namespace)
                    .get_opt(&policy.spec.application_ref)
                    .await
                    .ok()
                    .flatten()
            }
        };
        let Some(application) = application else {
            return false;
        };
        let recorded = application
            .status
            .as_ref()
            .and_then(|s| s.instance.as_ref());
        let bound: Option<BoundInstance> = match recorded {
            Some(bound) => Some(bound.clone()),
            None => lookup
                .directory
                .resolve(Some(namespace), Some(&application.spec.instance_ref), None)
                .await
                .ok(),
        };
        bound.is_some_and(|b| !b.governed_by_policy())
    }
}

#[async_trait::async_trait]
impl Admission for WebhookState {
    type Reason = ReasonCode;

    async fn admit(
        &self,
        kind: &str,
        namespace: &str,
        object: &DynamicObject,
    ) -> Result<(), Denial<ReasonCode>> {
        // The ValidatingWebhookConfiguration should only ever route
        // AuthentikApplication/AuthentikAccessPolicy here — an
        // unrecognized kind fails closed rather than assuming it's fine.
        let Some(kind) = parse_kind(kind) else {
            return Err(Denial::new(
                ReasonCode::NamespaceNotAllowed,
                format!("unrecognized kind for this webhook: {kind}"),
            ));
        };
        if self.bound_locally(kind, namespace, object).await {
            return Ok(());
        }
        let result = enforce(self.policies.as_ref(), namespace, kind)
            .await
            .map_err(|err| {
                tracing::error!(error = %err, "failed to list AuthentikNamespacePolicy");
                // failurePolicy: Fail already blocks the request if this
                // webhook errors; this denial is the same fail-closed intent.
                Denial::new(
                    ReasonCode::NamespaceNotAllowed,
                    "AuthentikNamespacePolicy lookup failed",
                )
            })?;
        if result.allowed {
            Ok(())
        } else {
            Err(Denial::new(
                result.reason.unwrap_or(ReasonCode::NamespaceNotAllowed),
                result.message.unwrap_or_else(|| "denied".to_string()),
            ))
        }
    }

    fn undecidable(&self) -> ReasonCode {
        ReasonCode::NamespaceNotAllowed
    }
}

fn parse_kind(kind: &str) -> Option<ResourceKind> {
    match kind {
        "AuthentikApplication" => Some(ResourceKind::AuthentikApplication),
        "AuthentikAccessPolicy" => Some(ResourceKind::AuthentikAccessPolicy),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use kube::core::admission::AdmissionReview;

    use super::*;

    /// A [`PolicySource`] that must never be read: `admit`'s
    /// unrecognized-`kind` branch returns before touching the policies.
    struct Unreachable;

    #[async_trait::async_trait]
    impl PolicySource<NamespaceRule> for Unreachable {
        async fn policies(
            &self,
        ) -> Result<Vec<Vec<NamespaceRule>>, application::ports::ClusterError> {
            unreachable!("an unrecognized kind is denied before any policy read")
        }
    }

    fn state_with_unreachable_policies() -> WebhookState {
        WebhookState {
            policies: Arc::new(Unreachable),
            instances: None,
        }
    }

    /// Every field a real `kube-apiserver` always sends on an admission
    /// request — `uid`/`kind`/`resource`/`operation`/`userInfo` have no
    /// serde default, so a realistic fixture needs all of them, unlike the
    /// pre-typed-refactor test payload which only had `uid`/`namespace`/
    /// `kind.kind`.
    fn review_with_kind(kind: &str) -> AdmissionReview<DynamicObject> {
        serde_json::from_value(serde_json::json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "11111111-1111-1111-1111-111111111111",
                "namespace": "default",
                "kind": { "group": "authentik.weebo.io", "version": "v1alpha1", "kind": kind },
                "resource": {
                    "group": "authentik.weebo.io",
                    "version": "v1alpha1",
                    "resource": "authentikapplications",
                },
                "operation": "CREATE",
                "userInfo": {},
                "object": {
                    "apiVersion": "authentik.weebo.io/v1alpha1", "kind": kind,
                    "metadata": {"name": "x", "namespace": "default"}, "spec": {},
                },
            }
        }))
        .expect("fixture must deserialize into AdmissionReview<DynamicObject>")
    }

    #[tokio::test]
    async fn unrecognized_kind_fails_closed() {
        let review = review_with_kind("SomeUnrelatedKind");

        let response =
            weebo_kit_runtime::admission::handle(&state_with_unreachable_policies(), review).await;

        let admission_response = response.response.expect("deny path always sets response");
        assert!(!admission_response.allowed);
        assert_eq!(
            admission_response.result.reason,
            ReasonCode::NamespaceNotAllowed.as_str()
        );
    }
}
