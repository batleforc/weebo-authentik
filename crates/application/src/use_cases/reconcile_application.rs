use api::AuthentikApplication;
use api::application::ProviderKind;
use domain::error::ReasonCode;

use crate::ports::{
    AuthentikGateway, GatewayError, Oauth2Credentials, SecretOwner, SecretStore, SecretStoreError,
};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};
use weebo_kit_api::SecretTarget;
use weebo_kit_application::emit;

/// Annotation authorizing a `spec.provider.kind` change on an existing
/// `AuthentikApplication`. Such a change deletes the old Authentik
/// provider and creates a new one of the new kind (oauth2 <-> proxy have
/// separate update endpoints, so the old id can't just be PATCHed against
/// the new kind) — this rotates `client_secret` for oauth2 and drops/
/// recreates outpost bindings for proxy, either of which can disrupt
/// active sessions. Absent, a kind change is refused rather than
/// attempted against the wrong provider type.
pub const ALLOW_DISRUPTIVE_UPDATE_ANNOTATION: &str = "authentik.weebo.io/allow-disruptive-update";

fn allows_disruptive_update(app: &AuthentikApplication) -> bool {
    api::ANNOTATIONS.allows_disruptive_update(&app.metadata)
}

/// Authentik's internal model name for a provider of this kind, as
/// returned by `RemoteApplication::provider_meta_model_name`. Only
/// oauth2/proxy are ever created by this operator — `reconcile_application`
/// rejects saml/ldap before this is reachable.
fn provider_kind_meta_model_name(kind: &ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Oauth2 => "authentik_providers_oauth2.oauth2provider",
        ProviderKind::Proxy => "authentik_providers_proxy.proxyprovider",
        ProviderKind::Saml | ProviderKind::Ldap => unreachable!("rejected before this is called"),
    }
}

fn provider_kind_from_meta_model_name(name: &str) -> Option<ProviderKind> {
    match name {
        "authentik_providers_oauth2.oauth2provider" => Some(ProviderKind::Oauth2),
        "authentik_providers_proxy.proxyprovider" => Some(ProviderKind::Proxy),
        _ => None,
    }
}

/// Whether `new_kind` differs from the live provider's kind, and if so,
/// whether the CR authorizes the resulting delete-recreate. Pure and unit
/// tested directly — see the tests below — independent of any gateway
/// call, since this is the actual safety decision the disruptive-update
/// gate exists to make.
#[derive(Debug, PartialEq, Eq)]
enum ProviderKindChange {
    /// No existing provider, or its kind already matches `new_kind`.
    None,
    Denied,
    Authorized,
}

fn classify_provider_kind_change(
    new_kind: &ProviderKind,
    existing_meta_model_name: Option<&str>,
    disruptive_update_allowed: bool,
) -> ProviderKindChange {
    let Some(existing) = existing_meta_model_name else {
        return ProviderKindChange::None;
    };
    if existing == provider_kind_meta_model_name(new_kind) {
        return ProviderKindChange::None;
    }
    if disruptive_update_allowed {
        ProviderKindChange::Authorized
    } else {
        ProviderKindChange::Denied
    }
}

/// Patch-first: diffs `spec` against the remote object read via
/// `authentik_id` and PATCHes; delete-recreate only happens for an
/// authorized `provider` variant change — see
/// `ALLOW_DISRUPTIVE_UPDATE_ANNOTATION`. See `.prompt/plan.md`, "Politique
/// de mutation".
///
/// oauth2 credentials go to every `spec.secretTargets` entry — or, when
/// there is none, to one target named after the CR, in its namespace, on
/// the instance's default backend ([`credential_targets`]).
pub async fn reconcile_application(
    app: &AuthentikApplication,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
) -> ReconcileOutcome {
    reconcile_application_recording(app, authentik_id, gateway, secrets)
        .await
        .outcome
}

/// What a reconcile of an application leaves to record: its outcome, and
/// the remote objects it knows to exist — recorded even when a later step
/// failed, so the next attempt patches them instead of creating them again
/// (and colliding with them).
#[derive(Debug)]
pub struct ApplicationReconciled {
    pub outcome: ReconcileOutcome,
    /// The application (`status.authentikId`), once created or updated by
    /// this reconcile; `None` leaves the recorded one as it was.
    pub authentik_id: Option<String>,
    /// The provider the application points at (`status.providerId`), once
    /// created or patched by this reconcile; `None` leaves the recorded one
    /// as it was.
    pub provider_id: Option<String>,
}

/// [`reconcile_application`], also returning what to record. The provider
/// recorded in `app.status.providerId` is the one patched when the
/// application was deleted out of band in Authentik: recreating the
/// application reuses it instead of leaving it behind.
pub async fn reconcile_application_recording(
    app: &AuthentikApplication,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
) -> ApplicationReconciled {
    let mut written = Written::default();
    let outcome = try_reconcile_application(app, authentik_id, gateway, secrets, &mut written)
        .await
        .unwrap_or_else(|failed| failed);
    ApplicationReconciled {
        outcome,
        authentik_id: written.application,
        provider_id: written.provider.map(|id| id.to_string()),
    }
}

/// The remote objects a reconcile has created or patched so far.
#[derive(Default)]
struct Written {
    application: Option<String>,
    provider: Option<i32>,
}

/// The provider recorded in the CR's status, if any.
pub fn recorded_provider_id(app: &AuthentikApplication) -> Option<i32> {
    app.status
        .as_ref()
        .and_then(|s| s.provider_id.as_deref())
        .and_then(|id| id.parse().ok())
}

/// Does the actual 5-step business flow (resolve existing provider ->
/// apply the disruptive-update gate -> upsert the provider -> upsert the
/// application -> attach the outpost for proxy). Returning
/// `Result<ReconcileOutcome, ReconcileOutcome>` lets every gateway/secret
/// call use `?` via `.map_err(errored_from_gateway_error)` instead of a
/// repeated `match { Ok(x) => x, Err(e) => return errored_from_gateway_error(e) }`.
async fn try_reconcile_application(
    app: &AuthentikApplication,
    authentik_id: Option<&str>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
    written: &mut Written,
) -> Result<ReconcileOutcome, ReconcileOutcome> {
    if matches!(
        app.spec.provider.kind,
        ProviderKind::Saml | ProviderKind::Ldap
    ) {
        return Err(ReconcileOutcome::Errored {
            reason: ReasonCode::UnsupportedProviderKind,
            message: format!(
                "provider kind {:?} is a schema-only stub, not implemented by any reconciler",
                app.spec.provider.kind
            ),
        });
    }

    // "Exactly one of oauth2/proxy set, matching kind" is schema-valid to
    // violate (kube-core can't express it as a hard constraint — see
    // `api::application::ProviderSpec`'s doc) and is "validated by the
    // reconciler, not the schema" per that same doc — this is that
    // validation. Without it, a schema-valid CR with `kind: oauth2` and no
    // `oauth2` field would panic below instead of erroring, unlike the
    // identical `SecretStoreSpec` pattern which `secret_store_factory.rs`
    // validates correctly.
    let spec_matches_kind = match app.spec.provider.kind {
        ProviderKind::Oauth2 => app.spec.provider.oauth2.is_some(),
        ProviderKind::Proxy => app.spec.provider.proxy.is_some(),
        ProviderKind::Saml | ProviderKind::Ldap => unreachable!("rejected above"),
    };
    if !spec_matches_kind {
        let field = match app.spec.provider.kind {
            ProviderKind::Oauth2 => "oauth2",
            ProviderKind::Proxy => "proxy",
            ProviderKind::Saml | ProviderKind::Ldap => unreachable!("rejected above"),
        };
        return Err(ReconcileOutcome::Errored {
            reason: ReasonCode::InvalidProviderSpec,
            message: format!(
                "spec.provider.kind is {:?} but spec.provider.{field} is not set",
                app.spec.provider.kind
            ),
        });
    }

    // On update, the current provider FK is read back off the live
    // Authentik application so the right provider gets patched in place
    // rather than a new one created. An application deleted out of band is
    // recreated (the CR is authoritative), pointing at the provider
    // recorded in the status if it still exists.
    let (authentik_id, existing) = match authentik_id {
        Some(id) => match gateway.get_application(id).await {
            Ok(remote) => (Some(id), Some(remote)),
            Err(GatewayError::Gone(_)) => (None, None),
            Err(e) => return Err(errored_from_gateway_error(e)),
        },
        None => (None, None),
    };
    let existing_meta_model_name = existing
        .as_ref()
        .and_then(|r| r.provider_meta_model_name.as_deref());

    let kind_change = classify_provider_kind_change(
        &app.spec.provider.kind,
        existing_meta_model_name,
        allows_disruptive_update(app),
    );

    if kind_change == ProviderKindChange::Denied {
        return Err(ReconcileOutcome::Errored {
            reason: ReasonCode::DisruptiveProviderKindChangeNotAllowed,
            message: format!(
                "spec.provider.kind is {:?} but Authentik's live provider is {:?} — changing \
                 it deletes and recreates the provider (rotates client_secret, may disrupt \
                 active sessions); add the {ALLOW_DISRUPTIVE_UPDATE_ANNOTATION:?} annotation \
                 set to \"true\" to authorize this",
                app.spec.provider.kind,
                existing_meta_model_name.unwrap_or("<unknown>"),
            ),
        });
    }

    if kind_change == ProviderKindChange::Authorized {
        // Both `Some`: `Authorized` is only ever returned when
        // `existing_meta_model_name` is `Some` (see
        // `classify_provider_kind_change`), and a `RemoteApplication` with
        // a recognized `provider_meta_model_name` always carries the
        // matching `provider_id` too.
        let old_kind = existing_meta_model_name.and_then(provider_kind_from_meta_model_name);
        if let (Some(old_provider_id), Some(old_kind)) =
            (existing.as_ref().and_then(|r| r.provider_id), old_kind)
        {
            gateway
                .delete_provider(old_provider_id, old_kind)
                .await
                .map_err(errored_from_gateway_error)?;
        }
    }

    // `None` on first reconcile (no `existing` at all, nothing recorded)
    // and right after an authorized kind change (the old provider was just
    // deleted above) — either way the upsert below must create, not patch.
    // Without a live application, or a live one pointing at no provider (its
    // provider deleted by a kind change whose update then failed), the
    // provider recorded in the status.
    let existing_provider_id: Option<i32> = if kind_change == ProviderKindChange::Authorized {
        None
    } else if let Some(remote) = &existing {
        remote.provider_id.or_else(|| recorded_provider_id(app))
    } else {
        recorded_provider_id(app)
    };

    // Recorded as soon as it exists (`written`), even if the application
    // then can't be created or updated to point at it: the next attempt
    // patches it rather than creating yet another one, and a cleanup
    // deletes it.
    let provider_id = upsert_provider(
        app,
        existing_provider_id,
        gateway,
        secrets,
        &mut written.provider,
    )
    .await?;

    // The PATCH carries `spec.slug`, and applications are slug-keyed: the
    // recorded id follows a slug change.
    let new_authentik_id = match authentik_id {
        Some(id) => gateway
            .update_application(id, app, provider_id)
            .await
            .map(|()| app.spec.slug.clone()),
        None => gateway.create_application(app, provider_id).await,
    }
    .map_err(errored_from_gateway_error)?;
    // Recorded before the outpost attachment: an application created here
    // and left unrecorded would make the next attempt collide with it.
    written.application = Some(new_authentik_id.clone());

    if let ProviderKind::Proxy = app.spec.provider.kind {
        let spec = app.spec.provider.proxy.as_ref().expect("checked above");
        gateway
            .attach_outpost(spec.outpost_ref.as_deref(), &provider_id.to_string())
            .await
            .map_err(|e| match e {
                GatewayError::NotFound(message) => ReconcileOutcome::Errored {
                    reason: ReasonCode::OutpostRefNotFound,
                    message,
                },
                other => errored_from_gateway_error(other),
            })?;
    }

    Ok(ReconcileOutcome::Synced {
        authentik_id: Some(new_authentik_id),
    })
}

/// Patches the provider `existing` designates, or creates one, and returns
/// its pk — also set in `upserted` as soon as the provider exists, before
/// the credentials are written.
///
/// `existing` reads as gone (`Gone`) through the spec's kind endpoint when
/// it was deleted out of band, or when it is of the other kind
/// (`spec.provider.kind` changed while the application itself was gone):
/// it is deleted whatever its kind — already gone is success — then a new
/// one is created.
async fn upsert_provider(
    app: &AuthentikApplication,
    existing: Option<i32>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
    upserted: &mut Option<i32>,
) -> Result<i32, ReconcileOutcome> {
    if let Some(id) = existing {
        match upsert_provider_once(app, Some(&id.to_string()), gateway, secrets, upserted).await {
            Ok(pk) => return Ok(pk),
            Err(UpsertError::Gateway(GatewayError::Gone(_))) => {
                gateway
                    .delete_any_provider(id)
                    .await
                    .map_err(errored_from_gateway_error)?;
            }
            Err(e) => return Err(e.into_outcome()),
        }
    }
    upsert_provider_once(app, None, gateway, secrets, upserted)
        .await
        .map_err(UpsertError::into_outcome)
}

/// A provider upsert's failure: the gateway's, kept as is so that a `Gone`
/// provider can be told apart, or an already mapped one.
enum UpsertError {
    Gateway(GatewayError),
    Outcome(ReconcileOutcome),
}

impl UpsertError {
    fn into_outcome(self) -> ReconcileOutcome {
        match self {
            UpsertError::Gateway(e) => errored_from_gateway_error(e),
            UpsertError::Outcome(outcome) => outcome,
        }
    }
}

async fn upsert_provider_once(
    app: &AuthentikApplication,
    existing: Option<&str>,
    gateway: &dyn AuthentikGateway,
    secrets: &dyn SecretStore,
    upserted: &mut Option<i32>,
) -> Result<i32, UpsertError> {
    match &app.spec.provider.kind {
        ProviderKind::Oauth2 => {
            let spec = app.spec.provider.oauth2.as_ref().expect(
                "kind=oauth2 implies oauth2 spec present, enforced by the reconciler contract",
            );
            let result = gateway
                .upsert_oauth2_provider(existing, &app.spec.name, &app.spec.slug, spec)
                .await
                .map_err(UpsertError::Gateway)?;
            let pk = parse_provider_id(&result.authentik_id).map_err(UpsertError::Outcome)?;
            *upserted = Some(pk);

            write_credentials(app, &result.credentials, secrets)
                .await
                .map_err(UpsertError::Outcome)?;
            Ok(pk)
        }
        ProviderKind::Proxy => {
            let spec = app.spec.provider.proxy.as_ref().expect(
                "kind=proxy implies proxy spec present, enforced by the reconciler contract",
            );
            let id = gateway
                .upsert_proxy_provider(existing, &app.spec.name, spec)
                .await
                .map_err(UpsertError::Gateway)?;
            let pk = parse_provider_id(&id).map_err(UpsertError::Outcome)?;
            *upserted = Some(pk);
            Ok(pk)
        }
        ProviderKind::Saml | ProviderKind::Ldap => unreachable!("rejected above"),
    }
}

fn parse_provider_id(id: &str) -> Result<i32, ReconcileOutcome> {
    id.parse().map_err(|_| ReconcileOutcome::Errored {
        reason: ReasonCode::AuthentikApiError,
        message: format!("unexpected non-integer provider id {id:?}"),
    })
}

/// `kind` recorded as the owner of every emitted credential.
pub const OWNER_KIND: &str = "AuthentikApplication";

fn cr_name(app: &AuthentikApplication) -> &str {
    app.metadata.name.as_deref().unwrap_or(&app.spec.slug)
}

/// The application, as the owner of the credentials it emits.
pub fn credentials_owner(app: &AuthentikApplication) -> SecretOwner<'_> {
    SecretOwner::namespaced(
        OWNER_KIND,
        app.metadata.namespace.as_deref().unwrap_or_default(),
        cr_name(app),
    )
}

/// `spec.secretTargets`, or the implicit target named after the CR.
pub fn credential_targets(
    app: &AuthentikApplication,
    secrets: &dyn SecretStore,
) -> Vec<SecretTarget> {
    emit::targets(&app.spec.secret_targets, cr_name(app), secrets)
}

/// Fans the credentials out to every target. Targets are checked first: an
/// application only ever writes into its own namespace (and, on Vault,
/// under `<pathPrefix>/<namespace>/`).
async fn write_credentials(
    app: &AuthentikApplication,
    credentials: &Oauth2Credentials,
    secrets: &dyn SecretStore,
) -> Result<(), ReconcileOutcome> {
    let owner = credentials_owner(app);
    let targets = credential_targets(app, secrets);
    let failed = |message: String| ReconcileOutcome::Errored {
        reason: ReasonCode::SecretStoreError,
        message,
    };
    emit::check_targets(&targets, &owner).map_err(|e| failed(e.to_string()))?;
    emit::write_all(&targets, &owner, &credentials.to_secret_data(), secrets)
        .await
        .map_err(|e| failed(format!("failed writing oauth2 credentials: {e}")))
}

/// Finalizer half: removes the credentials from every target this
/// application owns (another owner's value is left alone).
pub async fn delete_credentials(
    app: &AuthentikApplication,
    secrets: &dyn SecretStore,
) -> Result<(), SecretStoreError> {
    let owner = credentials_owner(app);
    emit::delete_all(&credential_targets(app, secrets), &owner, secrets).await
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    use api::application::{AuthentikApplicationSpec, ProviderSpec};
    use api::status::AuthentikStatus;
    use kube::api::ObjectMeta;

    use super::*;
    use crate::ports::{GatewayError, Oauth2ProviderUpsertResult, RemoteApplication};
    use crate::test_support::FakeGateway;
    use weebo_kit_application::test_support::FakeSecretStore;

    fn oauth2_spec() -> ProviderSpec {
        ProviderSpec {
            kind: ProviderKind::Oauth2,
            oauth2: Some(api::application::Oauth2ProviderSpec {
                client_type: Default::default(),
                client_id: None,
                authorization_flow: "default-authorization-flow".to_string(),
                invalidation_flow: "default-invalidation-flow".to_string(),
                signing_key: None,
                allowed_redirect_uris: vec![],
                property_mappings: vec![],
                grant_types: vec![],
                access_token_validity: None,
            }),
            proxy: None,
        }
    }

    fn proxy_spec() -> ProviderSpec {
        ProviderSpec {
            kind: ProviderKind::Proxy,
            oauth2: None,
            proxy: Some(api::application::ProxyProviderSpec {
                internal_host: "http://harbor.default.svc".to_string(),
                external_host: "https://harbor.example.com".to_string(),
                authorization_flow: "default-authorization-flow".to_string(),
                invalidation_flow: "default-invalidation-flow".to_string(),
                outpost_ref: None,
            }),
        }
    }

    fn app(
        provider: ProviderSpec,
        annotations: Option<BTreeMap<String, String>>,
    ) -> AuthentikApplication {
        AuthentikApplication {
            metadata: ObjectMeta {
                name: Some("harbor".to_string()),
                namespace: Some("team-a".to_string()),
                annotations,
                ..Default::default()
            },
            spec: AuthentikApplicationSpec {
                instance_ref: api::application::InstanceRef::named("prod"),
                name: "Harbor".to_string(),
                slug: "harbor".to_string(),
                meta_icon: None,
                provider,
                secret_targets: Vec::new(),
            },
            status: Some(AuthentikStatus {
                authentik_id: Some("harbor".to_string()),
                ..Default::default()
            }),
        }
    }

    fn allow_disruptive_annotation() -> BTreeMap<String, String> {
        BTreeMap::from([(
            ALLOW_DISRUPTIVE_UPDATE_ANNOTATION.to_string(),
            "true".to_string(),
        )])
    }

    // --- classify_provider_kind_change: the pure gate decision ---

    #[test]
    fn the_annotation_is_the_kits() {
        assert_eq!(
            api::ANNOTATIONS.key(weebo_kit_api::annotations::ALLOW_DISRUPTIVE_UPDATE),
            ALLOW_DISRUPTIVE_UPDATE_ANNOTATION
        );
    }

    #[test]
    fn no_existing_provider_is_not_a_kind_change() {
        assert_eq!(
            classify_provider_kind_change(&ProviderKind::Oauth2, None, false),
            ProviderKindChange::None
        );
    }

    #[test]
    fn matching_kind_is_not_a_kind_change() {
        assert_eq!(
            classify_provider_kind_change(
                &ProviderKind::Oauth2,
                Some("authentik_providers_oauth2.oauth2provider"),
                false
            ),
            ProviderKindChange::None
        );
    }

    #[test]
    fn differing_kind_without_the_annotation_is_denied() {
        assert_eq!(
            classify_provider_kind_change(
                &ProviderKind::Proxy,
                Some("authentik_providers_oauth2.oauth2provider"),
                false
            ),
            ProviderKindChange::Denied
        );
    }

    #[test]
    fn differing_kind_with_the_annotation_is_authorized() {
        assert_eq!(
            classify_provider_kind_change(
                &ProviderKind::Proxy,
                Some("authentik_providers_oauth2.oauth2provider"),
                true
            ),
            ProviderKindChange::Authorized
        );
    }

    // --- reconcile_application: the gate wired into production behavior ---

    #[tokio::test]
    async fn kind_change_without_the_annotation_is_denied_before_touching_the_gateway() {
        // FakeGateway's other fields are all None — if reconcile_application
        // called delete_provider/upsert_proxy_provider/etc. before the gate
        // returned, this test would panic naming the unscripted call.
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(5),
                provider_meta_model_name: Some(
                    "authentik_providers_oauth2.oauth2provider".to_string(),
                ),
            })),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::DisruptiveProviderKindChangeNotAllowed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn authorized_kind_change_deletes_the_old_provider_then_creates_the_new_one() {
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(5),
                provider_meta_model_name: Some(
                    "authentik_providers_oauth2.oauth2provider".to_string(),
                ),
            })),
            delete_provider_result: Some(Ok(())),
            upsert_proxy_provider_result: Some(Ok("77".to_string())),
            update_result: Some(Ok(())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        // Target kind is proxy, so secrets is never touched — FakeSecretStore
        // would panic if it were.
        let outcome = reconcile_application(
            &app(proxy_spec(), Some(allow_disruptive_annotation())),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "harbor"
        ));
    }

    // --- InvalidProviderSpec guard: schema-valid but kind/field mismatched ---

    #[tokio::test]
    async fn kind_oauth2_with_no_oauth2_spec_errors_instead_of_panicking() {
        let mismatched = ProviderSpec {
            kind: ProviderKind::Oauth2,
            oauth2: None,
            proxy: None,
        };
        // No FakeGateway field is scripted — this must return before ever
        // touching the gateway, let alone panicking on the `.expect()`s
        // further down that assume `oauth2` is `Some`.
        let gateway = FakeGateway::default();

        let outcome = reconcile_application(
            &app(mismatched, None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::InvalidProviderSpec,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn kind_proxy_with_no_proxy_spec_errors_instead_of_panicking() {
        let mismatched = ProviderSpec {
            kind: ProviderKind::Proxy,
            oauth2: None,
            proxy: None,
        };
        let gateway = FakeGateway::default();

        let outcome = reconcile_application(
            &app(mismatched, None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::InvalidProviderSpec,
                ..
            }
        ));
    }

    // --- saml/ldap rejection ---

    #[tokio::test]
    async fn saml_and_ldap_kinds_are_rejected_as_unsupported() {
        for kind in [ProviderKind::Saml, ProviderKind::Ldap] {
            let spec = ProviderSpec {
                kind,
                oauth2: None,
                proxy: None,
            };
            let gateway = FakeGateway::default();

            let outcome = reconcile_application(
                &app(spec, None),
                None,
                &gateway,
                &FakeSecretStore::default(),
            )
            .await;

            assert!(matches!(
                outcome,
                ReconcileOutcome::Errored {
                    reason: ReasonCode::UnsupportedProviderKind,
                    ..
                }
            ));
        }
    }

    // --- plain create/update success paths (no kind change involved) ---

    #[tokio::test]
    async fn first_reconcile_creates_the_oauth2_provider_and_writes_the_secret() {
        let gateway = FakeGateway {
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "42".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            create_result: Some(Ok("harbor".to_string())),
            ..Default::default()
        };
        let secrets = FakeSecretStore::default();

        let outcome =
            reconcile_application(&app(oauth2_spec(), None), None, &gateway, &secrets).await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "harbor"
        ));
    }

    #[tokio::test]
    async fn subsequent_reconcile_updates_the_oauth2_provider_in_place() {
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(42),
                provider_meta_model_name: Some(
                    "authentik_providers_oauth2.oauth2provider".to_string(),
                ),
            })),
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "42".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            update_result: Some(Ok(())),
            ..Default::default()
        };
        let secrets = FakeSecretStore::default();

        let outcome = reconcile_application(
            &app(oauth2_spec(), None),
            Some("harbor"),
            &gateway,
            &secrets,
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "harbor"
        ));
    }

    // --- where the credentials go ---

    fn oauth2_gateway() -> FakeGateway {
        FakeGateway {
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "42".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            create_result: Some(Ok("harbor".to_string())),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn credentials_go_to_the_implicit_target_owned_by_the_application() {
        let secrets = FakeSecretStore::default();
        reconcile_application(&app(oauth2_spec(), None), None, &oauth2_gateway(), &secrets).await;

        let data = secrets
            .get("team-a/harbor")
            .expect("implicit target written");
        assert_eq!(data["AUTHENTIK_CLIENT_ID"], "client");
        assert_eq!(data["AUTHENTIK_CLIENT_SECRET"], "secret");
        assert_eq!(data["AUTHENTIK_URL"], "https://authentik.example.com");
        assert_eq!(
            secrets.owner_of("team-a/harbor").as_deref(),
            Some("AuthentikApplication/team-a/harbor")
        );
    }

    #[tokio::test]
    async fn credentials_fan_out_to_every_target() {
        let mut application = app(oauth2_spec(), None);
        application.spec.secret_targets = ["a", "b"]
            .map(|name| SecretTarget {
                backend: Default::default(),
                name: name.to_string(),
                namespace: None,
                path: None,
            })
            .to_vec();
        let secrets = FakeSecretStore::default();
        reconcile_application(&application, None, &oauth2_gateway(), &secrets).await;

        assert!(secrets.get("team-a/a").is_some());
        assert!(secrets.get("team-a/b").is_some());
        assert_eq!(secrets.get("team-a/harbor"), None);
    }

    #[tokio::test]
    async fn a_target_in_another_namespace_is_refused_before_any_write() {
        let mut application = app(oauth2_spec(), None);
        application.spec.secret_targets = vec![SecretTarget {
            backend: Default::default(),
            name: "stolen".to_string(),
            namespace: Some("kube-system".to_string()),
            path: None,
        }];
        let secrets = FakeSecretStore::default();
        let outcome = reconcile_application(&application, None, &oauth2_gateway(), &secrets).await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::SecretStoreError,
                ..
            }
        ));
        assert_eq!(secrets.get("kube-system/stolen"), None);
    }

    #[tokio::test]
    async fn another_owners_secret_is_neither_overwritten_nor_deleted() {
        let theirs = BTreeMap::from([("k".to_string(), "v".to_string())]);
        let secrets = FakeSecretStore::default();
        secrets.insert_foreign(
            "team-a/harbor",
            "AuthentikApplication/team-a/other",
            theirs.clone(),
        );
        let application = app(oauth2_spec(), None);

        let outcome = reconcile_application(&application, None, &oauth2_gateway(), &secrets).await;
        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::SecretStoreError,
                ..
            }
        ));
        delete_credentials(&application, &secrets).await.unwrap();
        assert_eq!(secrets.get("team-a/harbor"), Some(theirs));
    }

    #[test]
    fn a_public_client_has_no_secret_key() {
        let data = Oauth2Credentials {
            client_id: "client".to_string(),
            client_secret: String::new(),
            authentik_url: "https://authentik.example.com".to_string(),
        }
        .to_secret_data();
        assert!(!data.contains_key("AUTHENTIK_CLIENT_SECRET"));
        assert_eq!(data.len(), 2);
    }

    // --- gateway/secret-store error propagation ---

    #[tokio::test]
    async fn get_application_error_maps_to_its_reason_code() {
        let gateway = FakeGateway {
            get_application_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn upsert_oauth2_provider_error_maps_to_its_reason_code() {
        let gateway = FakeGateway {
            upsert_oauth2_provider_result: Some(Err(GatewayError::AlreadyExists(
                "dup".to_string(),
            ))),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(oauth2_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn upsert_proxy_provider_error_maps_to_its_reason_code() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn secret_store_write_failure_maps_to_secret_store_error() {
        let gateway = FakeGateway {
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "42".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            ..Default::default()
        };
        let secrets = FakeSecretStore::default();
        secrets.fail_writes(true);

        let outcome =
            reconcile_application(&app(oauth2_spec(), None), None, &gateway, &secrets).await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::SecretStoreError,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn non_integer_oauth2_provider_id_errors_instead_of_panicking() {
        let gateway = FakeGateway {
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "not-a-number".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            ..Default::default()
        };
        let secrets = FakeSecretStore::default();

        let outcome =
            reconcile_application(&app(oauth2_spec(), None), None, &gateway, &secrets).await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn non_integer_proxy_provider_id_errors_instead_of_panicking() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("not-a-number".to_string())),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }

    // --- attach_outpost error handling (proxy-only tail step) ---

    #[tokio::test]
    async fn attach_outpost_not_found_maps_to_outpost_ref_not_found() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("55".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Err(GatewayError::NotFound("outpost".to_string()))),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::OutpostRefNotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn attach_outpost_other_error_maps_to_the_generic_reason_code() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("55".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikApiError,
                ..
            }
        ));
    }

    // --- the remote object deleted out of band, slug changes, rollbacks ---

    #[tokio::test]
    async fn an_application_deleted_out_of_band_is_recreated() {
        let gateway = FakeGateway {
            get_application_result: Some(Err(GatewayError::Gone("harbor".to_string()))),
            upsert_proxy_provider_result: Some(Ok("56".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &app(proxy_spec(), None),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "harbor"
        ));
    }

    #[tokio::test]
    async fn a_slug_change_moves_the_recorded_id_to_the_new_slug() {
        let mut renamed = app(proxy_spec(), None);
        renamed.spec.slug = "registry".to_string();
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(56),
                provider_meta_model_name: Some(
                    "authentik_providers_proxy.proxyprovider".to_string(),
                ),
            })),
            upsert_proxy_provider_result: Some(Ok("56".to_string())),
            update_result: Some(Ok(())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let outcome = reconcile_application(
            &renamed,
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            outcome,
            ReconcileOutcome::Synced { authentik_id: Some(id) } if id == "registry"
        ));
    }

    #[tokio::test]
    async fn a_provider_created_for_an_application_that_fails_is_recorded() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("57".to_string())),
            create_result: Some(Err(GatewayError::AlreadyExists("slug".to_string()))),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::AuthentikObjectAlreadyExists,
                ..
            }
        ));
        assert!(gateway.deleted_providers.lock().unwrap().is_empty());
        assert_eq!(
            reconciled.provider_id.as_deref(),
            Some("57"),
            "the next attempt patches it instead of creating another one"
        );
        assert_eq!(reconciled.authentik_id, None);
    }

    #[tokio::test]
    async fn an_existing_provider_is_kept_when_the_update_fails() {
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(56),
                provider_meta_model_name: Some(
                    "authentik_providers_proxy.proxyprovider".to_string(),
                ),
            })),
            upsert_proxy_provider_result: Some(Ok("56".to_string())),
            update_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };

        reconcile_application(
            &app(proxy_spec(), None),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(gateway.deleted_providers.lock().unwrap().is_empty());
    }

    // --- the provider recorded in the status ---

    fn with_recorded_provider(mut app: AuthentikApplication, id: &str) -> AuthentikApplication {
        app.status
            .as_mut()
            .expect("app() sets a status")
            .provider_id = Some(id.to_string());
        app
    }

    #[tokio::test]
    async fn a_synced_application_records_its_provider() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("56".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Synced { .. }
        ));
        assert_eq!(reconciled.provider_id.as_deref(), Some("56"));
    }

    #[tokio::test]
    async fn an_application_deleted_out_of_band_reuses_the_recorded_provider() {
        let gateway = FakeGateway {
            get_application_result: Some(Err(GatewayError::Gone("harbor".to_string()))),
            upsert_proxy_provider_result: Some(Ok("56".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &with_recorded_provider(app(proxy_spec(), None), "56"),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Synced { authentik_id: Some(ref id) } if id == "harbor"
        ));
        assert_eq!(
            *gateway.upserted_providers.lock().unwrap(),
            vec![Some("56".to_string())],
            "the recorded provider is patched, not a new one created"
        );
        assert_eq!(reconciled.provider_id.as_deref(), Some("56"));
    }

    #[tokio::test]
    async fn a_recorded_provider_deleted_out_of_band_too_is_created_again() {
        let gateway = FakeGateway {
            get_application_result: Some(Err(GatewayError::Gone("harbor".to_string()))),
            upsert_proxy_provider_sequence: Mutex::new(VecDeque::from([
                Err(GatewayError::Gone("56".to_string())),
                Ok("57".to_string()),
            ])),
            delete_any_provider_result: Some(Ok(())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &with_recorded_provider(app(proxy_spec(), None), "56"),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Synced { .. }
        ));
        assert_eq!(
            *gateway.upserted_providers.lock().unwrap(),
            vec![Some("56".to_string()), None]
        );
        assert_eq!(reconciled.provider_id.as_deref(), Some("57"));
    }

    #[tokio::test]
    async fn a_live_application_s_provider_wins_over_the_recorded_one() {
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: Some(58),
                provider_meta_model_name: Some(
                    "authentik_providers_proxy.proxyprovider".to_string(),
                ),
            })),
            upsert_proxy_provider_result: Some(Ok("58".to_string())),
            update_result: Some(Ok(())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &with_recorded_provider(app(proxy_spec(), None), "56"),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert_eq!(
            *gateway.upserted_providers.lock().unwrap(),
            vec![Some("58".to_string())]
        );
        assert_eq!(reconciled.provider_id.as_deref(), Some("58"));
    }

    #[tokio::test]
    async fn a_reconcile_failing_before_any_provider_records_nothing() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Err(GatewayError::Api("down".to_string()))),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored { .. }
        ));
        assert_eq!(reconciled.provider_id, None);
        assert_eq!(reconciled.authentik_id, None);
    }

    #[tokio::test]
    async fn an_application_created_before_a_failing_step_is_recorded() {
        let gateway = FakeGateway {
            upsert_proxy_provider_result: Some(Ok("57".to_string())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Err(GatewayError::Api("outpost down".to_string()))),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &app(proxy_spec(), None),
            None,
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored { .. }
        ));
        assert_eq!(
            reconciled.authentik_id.as_deref(),
            Some("harbor"),
            "the next attempt updates it instead of colliding with it"
        );
        assert_eq!(reconciled.provider_id.as_deref(), Some("57"));
    }

    #[tokio::test]
    async fn a_provider_whose_credentials_fail_to_be_written_is_recorded() {
        let gateway = FakeGateway {
            upsert_oauth2_provider_result: Some(Ok(Oauth2ProviderUpsertResult {
                authentik_id: "42".to_string(),
                credentials: Oauth2Credentials {
                    client_id: "client".to_string(),
                    client_secret: "secret".to_string(),
                    authentik_url: "https://authentik.example.com".to_string(),
                },
            })),
            ..Default::default()
        };
        let secrets = FakeSecretStore::default();
        secrets.fail_writes(true);

        let reconciled =
            reconcile_application_recording(&app(oauth2_spec(), None), None, &gateway, &secrets)
                .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored {
                reason: ReasonCode::SecretStoreError,
                ..
            }
        ));
        assert_eq!(reconciled.provider_id.as_deref(), Some("42"));
    }

    #[tokio::test]
    async fn a_recorded_provider_of_the_other_kind_is_deleted_before_a_new_one() {
        // Recorded as an oauth2 provider; the application was deleted out
        // of band and the spec now names a proxy provider: 56 reads as gone
        // through the proxy endpoint, though it still exists.
        let gateway = FakeGateway {
            get_application_result: Some(Err(GatewayError::Gone("harbor".to_string()))),
            upsert_proxy_provider_sequence: Mutex::new(VecDeque::from([
                Err(GatewayError::Gone("56".to_string())),
                Ok("57".to_string()),
            ])),
            delete_any_provider_result: Some(Ok(())),
            create_result: Some(Ok("harbor".to_string())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &with_recorded_provider(app(proxy_spec(), None), "56"),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Synced { .. }
        ));
        assert_eq!(*gateway.deleted_any_providers.lock().unwrap(), vec![56]);
        assert_eq!(reconciled.provider_id.as_deref(), Some("57"));
    }

    #[tokio::test]
    async fn a_live_application_without_provider_patches_the_recorded_one() {
        // A kind change deleted its provider, then the update pointing it
        // at the new one failed: that new one was recorded.
        let gateway = FakeGateway {
            get_application_result: Some(Ok(RemoteApplication {
                provider_id: None,
                provider_meta_model_name: None,
            })),
            upsert_proxy_provider_result: Some(Ok("57".to_string())),
            update_result: Some(Ok(())),
            attach_outpost_result: Some(Ok(())),
            ..Default::default()
        };

        let reconciled = reconcile_application_recording(
            &with_recorded_provider(app(proxy_spec(), None), "57"),
            Some("harbor"),
            &gateway,
            &FakeSecretStore::default(),
        )
        .await;

        assert_eq!(
            *gateway.upserted_providers.lock().unwrap(),
            vec![Some("57".to_string())]
        );
        assert_eq!(reconciled.provider_id.as_deref(), Some("57"));
    }
}
