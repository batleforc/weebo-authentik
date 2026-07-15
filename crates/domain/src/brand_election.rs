//! Pure implementation of the `AuthentikBrand` default-election algorithm.
//! Resolved entirely from Kubernetes-side state (creation timestamps of the
//! CRs), never by racing against the remote Authentik API. See
//! `.prompt/plan.md`, "Election du brand par defaut".

#[derive(Debug, Clone)]
pub struct BrandCandidate {
    pub name: String,
    /// `metadata.creationTimestamp`, as unix seconds.
    pub creation_timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Election {
    /// This CR is the earliest to claim `spec.default: true` — it's the
    /// only one that should touch the Authentik API.
    Winner,
    /// Another CR won the election; this one goes to `Ready: False`,
    /// `reason: DefaultBrandConflict`, and must not call the Authentik API.
    Loser { winner_name: String },
}

/// Elects a single winner among every `AuthentikBrand` CR currently
/// claiming `spec.default: true`. Earliest `creation_timestamp` wins, ties
/// broken by name. Returns one entry per candidate, order-preserving over
/// the input (not sorted) so callers can zip results back onto their CRs.
pub fn elect(candidates: &[BrandCandidate]) -> Vec<(String, Election)> {
    let Some(winner) = candidates
        .iter()
        .min_by(|a, b| {
            a.creation_timestamp
                .cmp(&b.creation_timestamp)
                .then_with(|| a.name.cmp(&b.name))
        })
        .cloned()
    else {
        return Vec::new();
    };

    candidates
        .iter()
        .map(|c| {
            if c.name == winner.name {
                (c.name.clone(), Election::Winner)
            } else {
                (
                    c.name.clone(),
                    Election::Loser {
                        winner_name: winner.name.clone(),
                    },
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, ts: i64) -> BrandCandidate {
        BrandCandidate {
            name: name.to_string(),
            creation_timestamp: ts,
        }
    }

    #[test]
    fn no_candidates_no_election() {
        assert_eq!(elect(&[]), Vec::new());
    }

    #[test]
    fn single_candidate_wins() {
        let result = elect(&[candidate("weebo", 100)]);
        assert_eq!(result, vec![("weebo".to_string(), Election::Winner)]);
    }

    #[test]
    fn earliest_creation_timestamp_wins() {
        let candidates = [candidate("late", 200), candidate("early", 100)];
        let result = elect(&candidates);
        assert_eq!(
            result,
            vec![
                (
                    "late".to_string(),
                    Election::Loser {
                        winner_name: "early".to_string()
                    }
                ),
                ("early".to_string(), Election::Winner),
            ]
        );
    }

    #[test]
    fn ties_broken_by_name() {
        let candidates = [candidate("zeta", 100), candidate("alpha", 100)];
        let result = elect(&candidates);
        assert_eq!(
            result,
            vec![
                (
                    "zeta".to_string(),
                    Election::Loser {
                        winner_name: "alpha".to_string()
                    }
                ),
                ("alpha".to_string(), Election::Winner),
            ]
        );
    }
}
