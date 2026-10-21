//! What the operator costs a cluster (weebo-kit-perf): the release binary
//! against envtest and a fake Authentik. A warm-up cycle, an idle
//! baseline, then a measured cycle — a burst of `AuthentikGroup`s, an idle
//! window with all of them in place, a spec churn on all of them, their
//! deletion — and a final idle window, checked against the kit's
//! small-cluster budget.
//!
//! `task perf` runs it. Tunable: `PERF_OBJECTS` (default 200),
//! `PERF_IDLE_SECS` (default 30); `WEEBO_PERF_REPORT_ONLY=1` reports
//! without failing; `WEEBO_PERF_BASELINE=<previous json>` adds the change
//! from a previous run. The report lands in `target/perf/`.

use std::time::Duration;

use api::group::AuthentikGroupSpec;
use api::instance::{AuthentikInstanceSpec, SecretKeyRef};
use api::{AuthentikGroup, AuthentikInstance};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, PostParams};
use weebo_kit_perf::{BASELINE, Budget, FINAL, OperatorMetrics, PerfRun, Workload, names};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The Authentik group the API answers with, under the pk it minted.
fn group_body(pk: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "pk": pk, "num_pk": 1, "name": name,
        "parents_obj": null, "users_obj": null, "roles_obj": [],
        "inherited_roles_obj": null, "children": [], "children_obj": null,
    })
}

/// `POST /core/groups/`: a fresh pk per group, like the real API.
struct CreateGroup;

impl Respond for CreateGroup {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let name = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|b| b.get("name")?.as_str().map(str::to_string))
            .unwrap_or_default();
        let pk = uuid_like(&name);
        ResponseTemplate::new(201).set_body_json(group_body(&pk, &name))
    }
}

/// `PATCH /core/groups/<pk>/`: echoes the pk.
struct UpdateGroup;

impl Respond for UpdateGroup {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let pk = request
            .url
            .path()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        ResponseTemplate::new(200).set_body_json(group_body(&pk, "group"))
    }
}

/// A stable UUID-shaped pk per name.
fn uuid_like(name: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    let n = h.finish();
    format!(
        "{:08x}-0000-4000-8000-{:012x}",
        (n >> 32) as u32,
        n & 0xffff_ffff_ffff
    )
}

async fn fake_authentik() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/core/groups/"))
        .respond_with(CreateGroup)
        .mount(&server)
        .await;
    // An update reads the group before writing what drifted. This fake
    // keeps no state: the name it echoes never matches, so every update
    // still writes — the load measured stays the read *and* the write.
    Mock::given(method("GET"))
        .and(path_regex(r"^/api/v3/core/groups/[^/]+/$"))
        .respond_with(UpdateGroup)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path_regex(r"^/api/v3/core/groups/[^/]+/$"))
        .respond_with(UpdateGroup)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/v3/core/groups/[^/]+/$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    server
}

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn group(name: &str) -> AuthentikGroup {
    AuthentikGroup {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        spec: AuthentikGroupSpec {
            name: name.to_string(),
            is_superuser: false,
            parent_ref: None,
            attributes: Default::default(),
        },
        status: None,
    }
}

/// `Ready: True` for the current generation.
fn converged(g: &AuthentikGroup) -> bool {
    let Some(status) = &g.status else {
        return false;
    };
    status.observed_generation == g.metadata.generation
        && status
            .conditions
            .iter()
            .any(|c| c.type_ == "Ready" && c.status == "True")
}

async fn burst(groups: &Workload<AuthentikGroup>, names: &[String], timeout: Duration) -> Duration {
    let objects: Vec<_> = names.iter().map(|n| group(n)).collect();
    let started = std::time::Instant::now();
    groups.create(&objects).await;
    groups.wait_all(names, timeout, converged).await;
    started.elapsed()
}

async fn churn(
    groups: &Workload<AuthentikGroup>,
    names: &[String],
    round: u32,
    timeout: Duration,
) -> Duration {
    let started = std::time::Instant::now();
    groups
        .patch_all(
            names,
            |_| serde_json::json!({"spec": {"attributes": {"perf-round": round.to_string()}}}),
        )
        .await;
    groups.wait_all(names, timeout, converged).await;
    started.elapsed()
}

async fn delete(
    groups: &Workload<AuthentikGroup>,
    names: &[String],
    timeout: Duration,
) -> Duration {
    let started = std::time::Instant::now();
    groups.delete_all(names).await;
    groups.wait_gone(names, timeout).await;
    started.elapsed()
}

/// Creates every object, idles with them in place (`loaded`, when `idle`
/// is set), changes every spec, deletes them all.
async fn cycle(
    run: &mut PerfRun,
    groups: &Workload<AuthentikGroup>,
    all: &[String],
    label: &str,
    round: u32,
    idle: Option<Duration>,
    timeout: Duration,
) {
    let n = all.len();
    run.load(&format!("{label} burst"), n, burst(groups, all, timeout))
        .await;
    if let Some(idle) = idle {
        run.idle(&format!("{label} loaded"), idle).await;
    }
    run.load(
        &format!("{label} churn"),
        n,
        churn(groups, all, round, timeout),
    )
    .await;
    run.load(&format!("{label} delete"), n, delete(groups, all, timeout))
        .await;
}

#[tokio::test]
#[ignore = "a perf run (minutes, release build): `task perf`"]
async fn the_operator_fits_a_small_cluster() {
    // An error only says another test of the binary set it up.
    tracing_subscriber::fmt().with_test_writer().try_init().ok();
    let objects = usize::try_from(env_or("PERF_OBJECTS", 200)).expect("PERF_OBJECTS fits a usize");
    let idle = Duration::from_secs(env_or("PERF_IDLE_SECS", 30));
    let timeout = Duration::from_secs(300);

    let cluster = testkit::envtest::start().await;
    let authentik = fake_authentik().await;
    let client = cluster.client();

    // The instance every group goes through (the cluster's only one).
    Api::<Secret>::namespaced(client.clone(), "default")
        .create(
            &PostParams::default(),
            &Secret {
                metadata: ObjectMeta {
                    name: Some("authentik-token".into()),
                    ..Default::default()
                },
                data: Some([("token".to_string(), ByteString(b"perf".to_vec()))].into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    Api::<AuthentikInstance>::all(client.clone())
        .create(
            &PostParams::default(),
            &AuthentikInstance {
                metadata: ObjectMeta {
                    name: Some("perf".into()),
                    ..Default::default()
                },
                spec: AuthentikInstanceSpec {
                    url: authentik.uri(),
                    token_secret_ref: SecretKeyRef {
                        name: "authentik-token".into(),
                        namespace: "default".into(),
                        key: "token".into(),
                    },
                    tls: Default::default(),
                    secret_store: Default::default(),
                },
                status: None,
            },
        )
        .await
        .unwrap();

    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_operator"));
    command.env("RUST_LOG", "warn");
    let mut run = PerfRun::start(
        "weebo-authentik",
        &cluster,
        command,
        OperatorMetrics::new("weebo_authentik"),
    )
    .await;
    let groups = run.workload(Api::<AuthentikGroup>::all(client.clone()));

    run.idle("startup", idle).await;
    // The warm-up is a whole cycle at full size: the baseline must follow a
    // process that already filled its caches, pools and allocator arenas to
    // what this load needs, or the next cycle's fill reads as a leak.
    let all = names("group", objects);
    cycle(&mut run, &groups, &all, "warm-up", 1, None, timeout).await;
    run.idle(BASELINE, idle).await;
    cycle(&mut run, &groups, &all, "cycle", 2, Some(idle), timeout).await;
    run.idle(FINAL, idle).await;

    run.finish().await.assert_within(&Budget::small_cluster());
}
