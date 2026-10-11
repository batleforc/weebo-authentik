//! Admission webhook: adapts an incoming `AdmissionReview` into a call to
//! `application::use_cases::evaluate_admission::enforce` (the kit's
//! allow-list engine, through `domain::allow_list`) — the same check the
//! reconcilers repeat.

use std::sync::Arc;

use api::namespace_policy::NamespaceRule;
use application::ports::PolicySource;
use application::use_cases::evaluate_admission::enforce;
use axum::Router;
use domain::allow_list::ResourceKind;
use domain::error::ReasonCode;
use kube::core::DynamicObject;
use weebo_kit_runtime::Health;
use weebo_kit_runtime::admission::{Admission, Denial};

#[derive(Clone)]
pub struct WebhookState {
    /// Every `AuthentikNamespacePolicy`, read on each request.
    pub policies: Arc<dyn PolicySource<NamespaceRule>>,
}

/// `POST /validate` (the kit's: a `DELETE`, or an `UPDATE` leaving `spec`
/// alone — the operator removing its finalizer — always passes), plus the
/// probes. Decisions are counted as `weebo_authentik_webhook_decisions_total`.
pub fn router(state: WebhookState, health: Arc<Health>) -> Router {
    weebo_kit_runtime::admission::router(
        Arc::new(state),
        health,
        "weebo-authentik-operator",
        "weebo_authentik",
    )
}

#[async_trait::async_trait]
impl Admission for WebhookState {
    type Reason = ReasonCode;

    async fn admit(
        &self,
        kind: &str,
        namespace: &str,
        _object: &DynamicObject,
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
