//! Resolves `AuthentikInstance` CRs for the gateway/secret-store
//! factories: the kit's resolver, served from a live reflector `Store`
//! when `operator::main` wires one in, from the API otherwise (tests).

use api::AuthentikInstance;
use kube::Client;
use kube::runtime::reflector::Store;

pub(crate) type InstanceResolver = weebo_kit_adapters::InstanceResolver<AuthentikInstance>;

pub(crate) fn live(client: Client) -> InstanceResolver {
    InstanceResolver::new(client, None)
}

pub(crate) fn with_store(client: Client, store: Store<AuthentikInstance>) -> InstanceResolver {
    InstanceResolver::new(client, Some(store))
}
