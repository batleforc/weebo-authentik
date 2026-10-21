//! `--apply`: adopts every CR the import wrote (the kit's `Adopter`) —
//! created paused (`authentik.weebo.io/import-pending`), its
//! `status.authentikId` seeded, then unpaused — so the operator never sees
//! it without the id of the object it stands for, and never creates a
//! duplicate next to it. `kubectl apply` can't do this: it drops `status`.

use std::fmt::Debug;
use std::path::Path;

use api::{
    AuthentikAccessPolicy, AuthentikApplication, AuthentikBrand, AuthentikFlow, AuthentikGroup,
    AuthentikOutpost, AuthentikScopeMapping, AuthentikStatus, AuthentikUser,
};
use kube::api::Api;
use kube::{Client, Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use weebo_kit_adapters::{Adopter, Adoption, Summary};

const ADOPTER: Adopter = Adopter {
    field_manager: "weebo-authentik-importer",
    annotations: api::ANNOTATIONS,
};

/// The file prefixes `write_cr` uses, in dependency order (groups before
/// the users and policies naming them…).
const ORDER: &[&str] = &[
    "authentikgroup",
    "authentikuser",
    "authentikflow",
    "authentikscopemapping",
    "authentikbrand",
    "authentikoutpost",
    "authentikapplication",
    "authentikaccesspolicy",
];

/// What the importer needs of each kind: its status, and an `Api` of the
/// right scope.
trait Seeded: Resource<DynamicType = ()> + Clone + Debug + Serialize + DeserializeOwned {
    fn seeded(&self) -> Option<&AuthentikStatus>;
    fn api(client: Client, namespace: Option<&str>) -> Api<Self>;
}

macro_rules! seeded {
    ($scope:ident: $($kind:ty),+) => {$(
        impl Seeded for $kind {
            fn seeded(&self) -> Option<&AuthentikStatus> {
                self.status.as_ref()
            }
            fn api(client: Client, namespace: Option<&str>) -> Api<Self> {
                seeded!(@$scope client, namespace)
            }
        }
    )+};
    (@cluster $client:ident, $ns:ident) => {{
        let _ = $ns;
        Api::all($client)
    }};
    (@namespaced $client:ident, $ns:ident) => {
        Api::namespaced($client, $ns.unwrap_or("default"))
    };
}
seeded!(cluster: AuthentikGroup, AuthentikUser, AuthentikFlow, AuthentikScopeMapping, AuthentikBrand, AuthentikOutpost);
seeded!(namespaced: AuthentikApplication, AuthentikAccessPolicy);

async fn adopt<K: Seeded>(client: &Client, cr: &K, force: bool) -> Result<Adoption, String> {
    let id = cr
        .seeded()
        .and_then(|s| s.authentik_id.clone())
        .ok_or("CR without a seeded status.authentikId")?;
    let api = K::api(client.clone(), cr.namespace().as_deref());
    ADOPTER
        .adopt(
            &api,
            cr,
            &id,
            json!({ "authentikId": id }),
            |existing: &K| existing.seeded().and_then(|s| s.authentik_id.clone()),
            force,
        )
        .await
        .map_err(|e| e.to_string())
}

async fn adopt_file<K: Seeded>(
    client: &Client,
    yaml: &str,
    force: bool,
) -> Result<Adoption, String> {
    let cr: K = serde_norway::from_str(yaml).map_err(|e| format!("parsing: {e}"))?;
    adopt(client, &cr, force).await
}

/// Adopts every CR file of `dir`, in dependency order; a failure is
/// reported on stderr and the rest carry on. `force` also takes over CRs
/// already in the cluster that are bound to nothing yet.
pub async fn apply_dir(client: &Client, dir: &Path, force: bool) -> anyhow::Result<Summary> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    files.sort();
    let mut summary = Summary::default();
    for prefix in ORDER {
        for path in files.iter().filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{prefix}-")))
        }) {
            let yaml = std::fs::read_to_string(path)?;
            let result = match *prefix {
                "authentikgroup" => adopt_file::<AuthentikGroup>(client, &yaml, force).await,
                "authentikuser" => adopt_file::<AuthentikUser>(client, &yaml, force).await,
                "authentikflow" => adopt_file::<AuthentikFlow>(client, &yaml, force).await,
                "authentikscopemapping" => {
                    adopt_file::<AuthentikScopeMapping>(client, &yaml, force).await
                }
                "authentikbrand" => adopt_file::<AuthentikBrand>(client, &yaml, force).await,
                "authentikoutpost" => adopt_file::<AuthentikOutpost>(client, &yaml, force).await,
                "authentikapplication" => {
                    adopt_file::<AuthentikApplication>(client, &yaml, force).await
                }
                _ => adopt_file::<AuthentikAccessPolicy>(client, &yaml, force).await,
            };
            match result {
                Ok(adoption) => summary.record(&Ok(adoption)),
                // A parse or not-seeded failure is not an `AdoptError`.
                Err(e) => {
                    eprintln!("error: {}: {e}", path.display());
                    summary.failed = summary.failed.saturating_add(1);
                }
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_written_kind_is_applied() {
        for kind in [
            "authentikgroup",
            "authentikuser",
            "authentikflow",
            "authentikscopemapping",
            "authentikbrand",
            "authentikoutpost",
            "authentikapplication",
            "authentikaccesspolicy",
        ] {
            assert!(ORDER.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn a_written_cr_parses_back_with_its_id() {
        let yaml = "apiVersion: authentik.weebo.io/v1alpha1\nkind: AuthentikGroup\n\
                    metadata:\n  name: admins\nspec:\n  name: admins\n\
                    status:\n  authentikId: '1234'\n";
        let cr: AuthentikGroup = serde_norway::from_str(yaml).unwrap();
        assert_eq!(cr.name_any(), "admins");
        assert_eq!(
            cr.seeded().and_then(|s| s.authentik_id.as_deref()),
            Some("1234")
        );
    }
}
