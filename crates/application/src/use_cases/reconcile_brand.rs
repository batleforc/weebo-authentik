//! Wires `domain::brand_election` to every `AuthentikBrand` CR currently
//! claiming `spec.default: true`. Election (`plan_brand_action`) is pure
//! Kubernetes-side logic, no I/O. `reconcile_brand` below does the actual
//! Authentik API sync: `BrandAction::Conflict` never reaches it (the
//! controller patches `Errored` straight from the election result,
//! matching `.prompt/plan.md`'s "sans appeler l'API Authentik" for
//! losing claimants) — only `NoDefaultClaim`/`BecomeDefault` do.

use api::AuthentikBrand;
use domain::brand_election::{self, BrandCandidate, Election};
use domain::error::ReasonCode;

use crate::ports::{AuthentikGateway, GatewayError};
use crate::use_cases::{ReconcileOutcome, errored_from_gateway_error};

pub struct BrandReconcileInput {
    pub name: String,
    pub creation_timestamp: i64,
    pub wants_default: bool,
}

pub enum BrandAction {
    /// Not claiming default — no election involved.
    NoDefaultClaim,
    /// This CR won the election.
    BecomeDefault,
    /// Another CR already holds default.
    Conflict {
        winner_name: String,
        reason: ReasonCode,
    },
}

pub fn plan_brand_action(
    this: &BrandReconcileInput,
    all_default_claimants: &[BrandReconcileInput],
) -> BrandAction {
    if !this.wants_default {
        return BrandAction::NoDefaultClaim;
    }

    let candidates: Vec<BrandCandidate> = all_default_claimants
        .iter()
        .filter(|c| c.wants_default)
        .map(|c| BrandCandidate {
            name: c.name.clone(),
            creation_timestamp: c.creation_timestamp,
        })
        .collect();

    match brand_election::elect(&candidates)
        .into_iter()
        .find(|(name, _)| name == &this.name)
    {
        Some((_, Election::Winner)) => BrandAction::BecomeDefault,
        Some((_, Election::Loser { winner_name })) => BrandAction::Conflict {
            winner_name,
            reason: ReasonCode::DefaultBrandConflict,
        },
        // Unreachable given the `wants_default` guard above: `this` is
        // always included in `all_default_claimants` by the caller.
        None => BrandAction::NoDefaultClaim,
    }
}

/// Patch-first sync of the brand object's own fields (domain, branding,
/// flows), plus the default toggle: set when `become_default`, cleared
/// otherwise if Authentik has this brand as its default.
/// Never called for `BrandAction::Conflict` — see the module doc comment.
pub async fn reconcile_brand(
    brand: &AuthentikBrand,
    authentik_id: Option<&str>,
    become_default: bool,
    gateway: &dyn AuthentikGateway,
) -> ReconcileOutcome {
    reconcile_brand_recording(brand, authentik_id, become_default, gateway)
        .await
        .outcome
}

/// What a reconcile of a brand leaves to record: its outcome, and the
/// brand it knows to exist — recorded even when the default toggle failed
/// after the brand was created, so the next attempt patches it instead of
/// creating it again (and colliding with it).
#[derive(Debug)]
pub struct BrandReconciled {
    pub outcome: ReconcileOutcome,
    /// The brand (`status.authentikId`), once created or updated by this
    /// reconcile; `None` leaves the recorded one as it was.
    pub authentik_id: Option<String>,
}

/// [`reconcile_brand`], also returning what to record.
pub async fn reconcile_brand_recording(
    brand: &AuthentikBrand,
    authentik_id: Option<&str>,
    become_default: bool,
    gateway: &dyn AuthentikGateway,
) -> BrandReconciled {
    let result = match authentik_id {
        // Deleted out of band: recreated, the CR is authoritative.
        Some(id) => match gateway.update_brand(id, brand).await {
            Err(GatewayError::Gone(_)) => gateway.create_brand(brand).await,
            result => result.map(|()| id.to_string()),
        },
        None => gateway.create_brand(brand).await,
    };

    let own_id = match result {
        Ok(id) => id,
        Err(e) => {
            return BrandReconciled {
                outcome: errored_from_gateway_error(e),
                authentik_id: None,
            };
        }
    };

    let outcome = match toggle_default(&own_id, become_default, gateway).await {
        Ok(()) => ReconcileOutcome::Synced {
            authentik_id: Some(own_id.clone()),
        },
        Err(e) => errored_from_gateway_error(e),
    };
    BrandReconciled {
        outcome,
        authentik_id: Some(own_id),
    }
}

/// Makes `own_id` Authentik's default brand (`become_default`), or makes
/// sure it no longer is.
async fn toggle_default(
    own_id: &str,
    become_default: bool,
    gateway: &dyn AuthentikGateway,
) -> Result<(), GatewayError> {
    let current = gateway.get_default_brand().await?;
    if become_default {
        // Already the default: nothing to write. Every reconcile used to
        // re-PATCH `default: true` onto it anyway, one of the two writes a
        // cycle the brand got (RFC 0003 of weebo-si).
        if current.as_deref() == Some(own_id) {
            return Ok(());
        }
        // Another brand is the default — see `.prompt/plan.md`, "Election
        // du brand par defaut", point 2: an unmanaged/native default gets
        // disabled before this CR's brand is enabled.
        if let Some(current) = current {
            gateway.set_brand_default(&current, false).await?;
        }
        gateway.set_brand_default(own_id, true).await
    } else {
        // The CR is authoritative both ways: a brand no longer claiming
        // default (`spec.default: true → false`, or adopted as the default
        // without claiming it) stops being Authentik's default. No other
        // brand is enabled in its place — see the cleanup's note on
        // point 4 of "Election du brand par defaut".
        if current.as_deref() == Some(own_id) {
            gateway.set_brand_default(own_id, false).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, ts: i64, wants_default: bool) -> BrandReconcileInput {
        BrandReconcileInput {
            name: name.to_string(),
            creation_timestamp: ts,
            wants_default,
        }
    }

    #[test]
    fn no_claim_short_circuits_without_electing() {
        let this = input("weebo", 100, false);
        let action = plan_brand_action(&this, &[]);
        assert!(matches!(action, BrandAction::NoDefaultClaim));
    }

    #[test]
    fn sole_claimant_becomes_default() {
        let this = input("weebo", 100, true);
        let action = plan_brand_action(&this, std::slice::from_ref(&this));
        assert!(matches!(action, BrandAction::BecomeDefault));
    }

    #[test]
    fn later_claimant_conflicts_with_earlier_winner() {
        let earlier = input("weebo", 100, true);
        let later = input("other", 200, true);
        let all = [earlier, later];
        let action = plan_brand_action(&all[1], &all);
        match action {
            BrandAction::Conflict {
                winner_name,
                reason,
            } => {
                assert_eq!(winner_name, "weebo");
                assert_eq!(reason, ReasonCode::DefaultBrandConflict);
            }
            _ => panic!("expected Conflict"),
        }
    }

    fn brand() -> AuthentikBrand {
        AuthentikBrand {
            metadata: kube::api::ObjectMeta {
                name: Some("weebo".to_string()),
                ..Default::default()
            },
            spec: serde_json::from_value(serde_json::json!({ "domain": "weebo.fr" }))
                .expect("a minimal brand spec"),
            status: None,
        }
    }

    #[tokio::test]
    async fn a_brand_no_longer_claiming_default_stops_being_the_default() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("b1".to_string()))),
            ..Default::default()
        };
        let outcome = reconcile_brand(&brand(), Some("b1"), false, &gateway).await;
        assert!(matches!(outcome, ReconcileOutcome::Synced { .. }));
        assert_eq!(
            *gateway.set_brand_default_calls.lock().unwrap(),
            vec![("b1".to_string(), false)]
        );
    }

    #[tokio::test]
    async fn the_default_brand_staying_default_writes_nothing() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("b1".to_string()))),
            ..Default::default()
        };
        let outcome = reconcile_brand(&brand(), Some("b1"), true, &gateway).await;
        assert!(matches!(outcome, ReconcileOutcome::Synced { .. }));
        assert!(gateway.set_brand_default_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn another_default_is_disabled_before_this_one_is_enabled() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("native".to_string()))),
            ..Default::default()
        };
        reconcile_brand(&brand(), Some("b1"), true, &gateway).await;
        assert_eq!(
            *gateway.set_brand_default_calls.lock().unwrap(),
            vec![("native".to_string(), false), ("b1".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn a_brand_not_claiming_default_leaves_another_default_alone() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("native".to_string()))),
            ..Default::default()
        };
        reconcile_brand(&brand(), Some("b1"), false, &gateway).await;
        assert!(gateway.set_brand_default_calls.lock().unwrap().is_empty());
    }

    /// Created, then the default toggle fails: the brand is returned all
    /// the same, so the controller records it and the next attempt patches
    /// it instead of colliding with it.
    #[tokio::test]
    async fn a_brand_created_before_a_failing_default_toggle_is_recorded() {
        let gateway = crate::test_support::FakeGateway {
            create_result: Some(Ok("b1".to_string())),
            get_default_brand_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };
        let reconciled = reconcile_brand_recording(&brand(), None, true, &gateway).await;
        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored { .. }
        ));
        assert_eq!(reconciled.authentik_id.as_deref(), Some("b1"));
        assert!(gateway.set_brand_default_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_brand_recreated_out_of_band_before_a_failing_toggle_is_recorded() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Err(GatewayError::Gone("b1".to_string()))),
            create_result: Some(Ok("b2".to_string())),
            get_default_brand_result: Some(Err(GatewayError::Api("boom".to_string()))),
            ..Default::default()
        };
        let reconciled = reconcile_brand_recording(&brand(), Some("b1"), false, &gateway).await;
        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored { .. }
        ));
        assert_eq!(reconciled.authentik_id.as_deref(), Some("b2"));
    }

    #[tokio::test]
    async fn a_failing_create_records_nothing() {
        let gateway =
            crate::test_support::FakeGateway::create(Err(GatewayError::Api("boom".to_string())));
        let reconciled = reconcile_brand_recording(&brand(), None, true, &gateway).await;
        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Errored { .. }
        ));
        assert_eq!(reconciled.authentik_id, None);
    }

    #[tokio::test]
    async fn the_winner_replaces_another_default() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("native".to_string()))),
            ..Default::default()
        };
        let reconciled = reconcile_brand_recording(&brand(), Some("b1"), true, &gateway).await;
        assert!(matches!(
            reconciled.outcome,
            ReconcileOutcome::Synced { authentik_id: Some(ref id) } if id == "b1"
        ));
        assert_eq!(
            *gateway.set_brand_default_calls.lock().unwrap(),
            vec![("native".to_string(), false), ("b1".to_string(), true)]
        );
    }
}
