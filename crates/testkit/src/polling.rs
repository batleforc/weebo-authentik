//! Status polling for envtest-backed controller tests: the kit's helpers,
//! parameterized over the timeout/poll interval so each caller keeps its
//! own tuning (the mocked tests can afford a short 15s/200ms; the
//! real-Authentik tests need more slack).

pub use weebo_kit_testkit::polling::{wait_for, wait_for_absence};
