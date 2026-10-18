//! Write only what differs (RFC 0003 of weebo-si, "weebo-authentik n'écrit
//! que ce qui diffère"). Every `update_*`/`upsert_*` of the gateway used to
//! PATCH its whole payload on every reconcile, five minutes apart, with
//! nothing changed: Authentik recorded each as a `model_updated` event,
//! fired tasks for it and, for a provider, rebuilt the outpost's
//! permissions. Now the gateway reads the remote object, and [`Drift`]
//! strips from the PATCH every field that already holds the desired value.
//! No field left: no call.
//!
//! The comparison is on the payload the gateway would have sent, so what
//! the operator owns does not change: a field it never sends (`None`) is
//! "no opinion" and is never compared, a field it sends is corrected when
//! it drifts, exactly as before — only without the write when it hasn't.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};

/// The fields of a PATCH whose desired value differs from the remote one —
/// the ones left in the request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drift(Vec<&'static str>);

impl Drift {
    /// Nothing differs: the PATCH can be skipped.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The drifted fields, in the order they were checked.
    pub fn fields(&self) -> &[&'static str] {
        &self.0
    }

    /// `desired` is the PATCH field (`None`: not sent, no opinion); `remote`
    /// the value read back. Equal: `desired` is cleared, the field is left
    /// out of the PATCH. Different: it stays, and `field` is recorded.
    pub fn keep<T: PartialEq>(&mut self, field: &'static str, desired: &mut Option<T>, remote: &T) {
        self.keep_with(field, desired, remote, |d, r| d == r);
    }

    /// [`Drift::keep`] against a remote value Authentik may leave unset:
    /// an unset remote value never equals a desired one.
    pub fn keep_opt<T: PartialEq>(
        &mut self,
        field: &'static str,
        desired: &mut Option<T>,
        remote: &Option<T>,
    ) {
        self.keep_with(field, desired, remote, |d, r| r.as_ref() == Some(d));
    }

    /// [`Drift::keep`] with the caller's equality — for a list compared as a
    /// set ([`same_set`]), a map compared on its desired keys only
    /// ([`same_subset`]), or a remote value of another type than the
    /// request's (`RedirectUri` vs `RedirectUriRequest`).
    pub fn keep_with<T, R>(
        &mut self,
        field: &'static str,
        desired: &mut Option<T>,
        remote: &R,
        same: impl FnOnce(&T, &R) -> bool,
    ) {
        if let Some(d) = desired.as_ref() {
            if same(d, remote) {
                *desired = None;
            } else {
                self.0.push(field);
            }
        }
    }
}

/// The same elements, whatever their order, duplicates counted — for the
/// lists Authentik treats as sets (groups, parents, property mappings,
/// grant types, redirect URIs). `PartialEq` only: these lists hold a
/// handful of elements, a quadratic count is cheaper than requiring `Ord`
/// of the client's every type.
pub fn same_set<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| a.iter().filter(|y| *y == x).count() == b.iter().filter(|y| *y == x).count())
}

/// Every desired key holds the desired value remotely; other remote keys
/// are ignored — for a map Authentik fills in on its own (an outpost's
/// `config`, which it completes with every default).
pub fn same_subset<K, V, S1, S2>(desired: &HashMap<K, V, S1>, remote: &HashMap<K, V, S2>) -> bool
where
    K: Hash + Eq,
    V: PartialEq,
    S2: BuildHasher,
{
    desired
        .iter()
        .all(|(k, v)| remote.get(k).is_some_and(|r| r == v))
}

/// Equal once leading and trailing whitespace is ignored — for a text field
/// Authentik trims before storing it (Django REST Framework's `CharField`
/// default, `trim_whitespace`): a CR's YAML block scalar (`|`) ends with a
/// `\n` that never comes back, and a byte-for-byte comparison drifted on
/// every reconcile (v0.18.2, every `AuthentikScopeMapping`'s `expression`).
/// The PATCH still carries the spec's value untouched; only the comparison
/// is done on the stored form.
pub fn same_trimmed(desired: &str, remote: &str) -> bool {
    desired.trim() == remote.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_equal_field_leaves_the_patch() {
        let mut drift = Drift::default();
        let mut name = Some("angos".to_string());
        drift.keep("name", &mut name, &"angos".to_string());
        assert_eq!(name, None);
        assert!(drift.is_empty());
    }

    #[test]
    fn a_different_field_stays_and_is_recorded() {
        let mut drift = Drift::default();
        let mut name = Some("angos".to_string());
        drift.keep("name", &mut name, &"harbor".to_string());
        assert_eq!(name.as_deref(), Some("angos"));
        assert_eq!(drift.fields(), ["name"]);
    }

    #[test]
    fn a_field_not_sent_is_no_opinion() {
        let mut drift = Drift::default();
        let mut icon: Option<String> = None;
        drift.keep_opt("meta_icon", &mut icon, &Some("x.png".to_string()));
        assert_eq!(icon, None);
        assert!(drift.is_empty());
    }

    #[test]
    fn an_unset_remote_value_differs_from_a_desired_one() {
        let mut drift = Drift::default();
        let mut validity = Some("hours=1".to_string());
        drift.keep_opt("access_token_validity", &mut validity, &None);
        assert_eq!(drift.fields(), ["access_token_validity"]);
    }

    #[test]
    fn a_nullable_reference_compares_its_inner_value() {
        // `Option<Option<Uuid>>` fields: `Some(None)` sends an explicit null.
        let mut drift = Drift::default();
        let mut flow: Option<Option<u8>> = Some(None);
        drift.keep_opt("flow_recovery", &mut flow, &Some(None));
        assert_eq!(flow, None);
        let mut flow: Option<Option<u8>> = Some(Some(1));
        drift.keep_opt("flow_recovery", &mut flow, &Some(None));
        assert_eq!(drift.fields(), ["flow_recovery"]);
    }

    #[test]
    fn sets_ignore_order_but_not_duplicates() {
        assert!(same_set(&[1, 2, 3], &[3, 1, 2]));
        assert!(!same_set(&[1, 2], &[1, 2, 3]));
        assert!(!same_set(&[1, 1, 2], &[1, 2, 2]));
        assert!(same_set::<u8>(&[], &[]));
    }

    #[test]
    fn a_subset_ignores_the_keys_authentik_added() {
        let remote = HashMap::from([("log_level", 1), ("authentik_host", 2)]);
        assert!(same_subset(&HashMap::from([("log_level", 1)]), &remote));
        assert!(!same_subset(&HashMap::from([("log_level", 3)]), &remote));
        assert!(!same_subset(&HashMap::from([("missing", 1)]), &remote));
    }

    #[test]
    fn trimmed_text_ignores_only_the_outer_whitespace() {
        assert!(same_trimmed("return {}\n", "return {}"));
        assert!(same_trimmed("  return {}\n\n", "return {}"));
        assert!(!same_trimmed("return  {}", "return {}"));
        assert!(!same_trimmed("return {}\nreturn 1\n", "return {}"));
    }

    #[test]
    fn several_fields_are_reported_in_order() {
        let mut drift = Drift::default();
        let (mut a, mut b, mut c) = (Some(1), Some(2), Some(3));
        drift.keep("a", &mut a, &9);
        drift.keep("b", &mut b, &2);
        drift.keep("c", &mut c, &9);
        assert_eq!(drift.fields(), ["a", "c"]);
        assert_eq!((a, b, c), (Some(1), None, Some(3)));
    }
}
