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
        Err(e) => return errored_from_gateway_error(e),
    };

    if become_default {
        // Only disable the currently-default brand if it isn't already
        // this one (idempotent re-reconciles shouldn't toggle it off and
        // back on) — see `.prompt/plan.md`, "Election du brand par
        // defaut", point 2: an unmanaged/native default gets disabled
        // before this CR's brand is enabled.
        match gateway.get_default_brand().await {
            Ok(Some(current)) if current != own_id => {
                if let Err(e) = gateway.set_brand_default(&current, false).await {
                    return errored_from_gateway_error(e);
                }
            }
            Ok(_) => {}
            Err(e) => return errored_from_gateway_error(e),
        }
        if let Err(e) = gateway.set_brand_default(&own_id, true).await {
            return errored_from_gateway_error(e);
        }
    } else {
        // The CR is authoritative both ways: a brand no longer claiming
        // default (`spec.default: true → false`, or adopted as the default
        // without claiming it) stops being Authentik's default. No other
        // brand is enabled in its place — see the cleanup's note on
        // point 4 of "Election du brand par defaut".
        match gateway.get_default_brand().await {
            Ok(Some(current)) if current == own_id => {
                if let Err(e) = gateway.set_brand_default(&own_id, false).await {
                    return errored_from_gateway_error(e);
                }
            }
            Ok(_) => {}
            Err(e) => return errored_from_gateway_error(e),
        }
    }

    ReconcileOutcome::Synced {
        authentik_id: Some(own_id),
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
    async fn a_brand_not_claiming_default_leaves_another_default_alone() {
        let gateway = crate::test_support::FakeGateway {
            update_result: Some(Ok(())),
            get_default_brand_result: Some(Ok(Some("native".to_string()))),
            ..Default::default()
        };
        reconcile_brand(&brand(), Some("b1"), false, &gateway).await;
        assert!(gateway.set_brand_default_calls.lock().unwrap().is_empty());
    }
}
