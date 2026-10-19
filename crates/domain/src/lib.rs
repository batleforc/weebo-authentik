pub mod allow_list;
pub mod brand_election;
pub mod error;
pub mod provider;

/// Write only what differs (weebo-si RFC 0003): the kit's comparison
/// helpers (`Drift`, `same_set`, `same_subset`, `same_trimmed`).
pub use weebo_kit_domain::drift;

/// A `Ready: False` cause, carrying one of this operator's reason codes.
pub type Failure = weebo_kit_domain::Failure<error::ReasonCode>;
/// A non-blocking condition shown next to `Ready: True`.
pub type Advisory = weebo_kit_domain::Advisory<error::ReasonCode>;
