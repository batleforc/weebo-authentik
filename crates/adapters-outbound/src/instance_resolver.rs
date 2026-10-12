//! The operator's instances, both kinds — `AuthentikInstance`
//! (cluster-scoped) and `AuthentikLocalInstance` (namespaced) — through the
//! kit's `Instances`: read from the binary's reflector `Store`s, or from the
//! API on every call (tests), on the operator's watch scope. It is also the
//! `InstanceDirectory` resolving a CR's `instanceRef`.

use api::{AuthentikInstance, AuthentikLocalInstance};

pub type AuthentikInstances =
    weebo_kit_adapters::Instances<AuthentikInstance, AuthentikLocalInstance>;
