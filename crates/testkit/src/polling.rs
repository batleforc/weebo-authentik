//! Generic status-polling helpers for envtest-backed controller tests.
//!
//! Both `adapters-inbound`'s wiremock-backed controller tests and
//! `operator`'s real-sidecar-Authentik tests poll a CR every N ms until some
//! condition holds (or it doesn't gone within a timeout) — this is that
//! shape, parameterized over the timeout/poll interval so each caller can
//! keep its own tuning (the mocked tests can afford a short 15s/200ms; the
//! real-Authentik tests need more slack).

use std::time::Duration;

/// Polls `api.get(name)` every `poll_interval` until `extract` returns
/// `Some`, up to `timeout`. Panics with the last-observed object (via its
/// `Debug` impl) if the timeout elapses first.
pub async fn wait_for<K, T, F>(
    api: &kube::Api<K>,
    name: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut extract: F,
) -> T
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
    F: FnMut(&K) -> Option<T>,
{
    let mut last: Option<K> = None;
    tokio::time::timeout(timeout, async {
        loop {
            let obj = api.get(name).await.expect("CR must be gettable");
            if let Some(v) = extract(&obj) {
                return v;
            }
            last = Some(obj);
            tokio::time::sleep(poll_interval).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("condition on {name:?} was not met within the {timeout:?} timeout; last observed: {last:?}")
    })
}

/// Polls `api.get(name)` every `poll_interval` until it errors (i.e. the
/// object is gone), up to `timeout` — the delete-then-wait-for-404 half of
/// the finalizer-cleanup pattern.
pub async fn wait_for_absence<K>(
    api: &kube::Api<K>,
    name: &str,
    timeout: Duration,
    poll_interval: Duration,
) where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    tokio::time::timeout(timeout, async {
        loop {
            if api.get(name).await.is_err() {
                return;
            }
            tokio::time::sleep(poll_interval).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{name:?} was not removed within the {timeout:?} timeout"))
}
