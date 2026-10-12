pub mod status;

/// API group shared by every CRD of this operator.
pub const GROUP: &str = "authentik.weebo.io";
/// The kit's common annotations under [`GROUP`]:
/// `authentik.weebo.io/force-orphan` (release a CR without cleanup),
/// `allow-disruptive-update`, `rotate-secret`, `import-pending` (the
/// importer is seeding the CR's status; the controllers leave it alone).
pub const ANNOTATIONS: weebo_kit_api::Annotations = weebo_kit_api::Annotations::new(GROUP);

/// Annotation naming the CR that emitted a `Secret` (`<kind>/<ns>/<name>`):
/// the operator never overwrites nor deletes a Secret owned by another CR.
pub const SECRET_OWNER_ANNOTATION: &str = "authentik.weebo.io/owner";

pub mod access_policy;
pub mod application;
pub mod brand;
pub mod flow;
pub mod group;
pub mod instance;
pub mod local_instance;
pub mod namespace_policy;
pub mod outpost;
pub mod scope_mapping;
pub mod user;

pub use access_policy::AuthentikAccessPolicy;
pub use application::AuthentikApplication;
pub use brand::AuthentikBrand;
pub use flow::AuthentikFlow;
pub use group::AuthentikGroup;
pub use instance::AuthentikInstance;
pub use local_instance::AuthentikLocalInstance;
pub use namespace_policy::AuthentikNamespacePolicy;
pub use outpost::AuthentikOutpost;
pub use scope_mapping::AuthentikScopeMapping;
pub use status::AuthentikStatus;
pub use user::AuthentikUser;
