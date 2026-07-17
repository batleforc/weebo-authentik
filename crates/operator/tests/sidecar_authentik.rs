//! Real-Authentik functional test — the layer-4 tier from `.prompt/plan.md`
//! ("E2E complet ... vraie instance Authentik jetable"), but scoped to
//! *this repo's Che devworkspace sidecar* (`devfile.yaml`'s
//! `authentik-db`/`authentik-server`/`authentik-worker` containers) rather
//! than a `kind`+throwaway-container cluster: no Docker/Podman is available
//! in this environment (see `CLAUDE.md`), so the sidecar containers already
//! sitting idle (`sleep infinity`) in this workspace's own pod are the
//! stand-in.
//!
//! What each test in this file actually does:
//! 1. Starts `ak server`/`ak worker` inside the idle sidecar containers
//!    (`kubectl exec`, backgrounded — they're normally left idle so a
//!    manual `task authentik:wipe-db` doesn't fight a running migration).
//! 2. Polls Authentik's own `/-/health/ready/` until it reports ready.
//! 3. Runs the *real* controller for one CRD (`envtest` control plane and
//!    the real `AuthentikHttpGateway`, no `wiremock`) against it —
//!    `group_controller.rs` in `adapters-inbound` is the mocked version of
//!    the group scenario; this is its real-HTTP counterpart, plus the same
//!    shape for `AuthentikUser`.
//! 4. Proves the round trip is genuine (not just a 201 that happened to
//!    deserialize) by deleting the CR, confirming the finalizer's real
//!    `delete_*` call landed, then recreating the same Authentik-side
//!    name: a second create only succeeds if the first was truly gone
//!    server-side, otherwise Authentik rejects the name collision and
//!    `status.authentikId` never gets set.
//!
//! Requires this repo's own `devfile.yaml` sidecars (`AUTHENTIK_BOOTSTRAP_TOKEN`
//! must match `BOOTSTRAP_TOKEN` below) — not something a generic CI runner
//! has, hence `#[ignore]`. Run explicitly via `task test:live-authentik`.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adapters_inbound::controller::{self, Ctx};
use adapters_outbound::{AuthentikHttpGateway, K8sSecretStore};
use api::brand::AuthentikBrandSpec;
use api::group::AuthentikGroupSpec;
use api::outpost::{AuthentikOutpostSpec, OutpostType};
use api::user::AuthentikUserSpec;
use api::{AuthentikBrand, AuthentikGroup, AuthentikOutpost, AuthentikStatus, AuthentikUser};
use kube::api::{Api, ObjectMeta, PostParams};
use testkit::envtest::EnvTestCluster;
use testkit::static_gateway_factory::StaticGatewayFactory;

/// Sidecar containers share this pod's network namespace (no per-container
/// DNS aliasing like docker-compose) — `localhost` is how the main
/// container reaches `authentik-server`'s port, once it's up.
const AUTHENTIK_BASE: &str = "http://127.0.0.1:9000";

/// Must match `AUTHENTIK_BOOTSTRAP_TOKEN` in `devfile.yaml` — that env var
/// is what makes this token exist on the akadmin user without any manual
/// setup-wizard step.
const BOOTSTRAP_TOKEN: &str = "dev-only-insecure-bootstrap-token-change-me";

const READY_TIMEOUT: Duration = Duration::from_secs(180);

fn workspace_pod_name() -> String {
    let workspace = std::env::var("WORKSPACE_NAME").expect(
        "WORKSPACE_NAME must be set — this test only makes sense inside \
         this repo's own Che devworkspace, whose devfile.yaml provides the \
         authentik-server/authentik-worker/authentik-db sidecars",
    );
    let out = Command::new("kubectl")
        .args([
            "get",
            "pod",
            "-l",
            &format!("controller.devfile.io/devworkspace_name={workspace}"),
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .output()
        .expect("kubectl must be available and able to list this workspace's own pod");
    assert!(
        out.status.success(),
        "kubectl get pod failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let name = String::from_utf8(out.stdout).expect("pod name must be utf-8");
    assert!(!name.is_empty(), "no pod found for workspace {workspace:?}");
    name
}

/// Starts `ak <subcommand>` in the background inside `container`, unless a
/// still-alive one from an earlier run is already there — `kubectl exec`'s
/// session ending doesn't kill it (no `nohup`-defeating SIGHUP), so repeat
/// test runs during a dev session reuse the same long-lived process rather
/// than paying Authentik's ~1min startup cost every time.
fn ensure_ak_running(pod: &str, container: &str, subcommand: &str) {
    let already_running = Command::new("kubectl")
        .args([
            "exec",
            "-c",
            container,
            pod,
            "--",
            "sh",
            "-c",
            "test -f /tmp/ak.pid && kill -0 \"$(cat /tmp/ak.pid)\" 2>/dev/null",
        ])
        .status();
    if matches!(already_running, Ok(s) if s.success()) {
        return;
    }

    let script =
        format!("nohup /lifecycle/ak {subcommand} >/tmp/ak.log 2>&1 & echo $! > /tmp/ak.pid");
    let status = Command::new("kubectl")
        .args(["exec", "-c", container, pod, "--", "sh", "-c", &script])
        .status()
        .unwrap_or_else(|e| {
            panic!("kubectl exec could not start `ak {subcommand}` in {container}: {e}")
        });
    assert!(
        status.success(),
        "kubectl exec exited non-zero starting `ak {subcommand}` in {container}"
    );
}

fn authentik_is_ready() -> bool {
    let out = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "3",
            &format!("{AUTHENTIK_BASE}/-/health/ready/"),
        ])
        .output();
    matches!(out, Ok(o) if o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim().starts_with('2'))
}

/// `/-/health/ready/` (checked by `authentik_is_ready`) only proves the
/// process is up and DB/redis-connected — observed in practice, a fresh
/// `ak server` boot can report ready several seconds before token-bearer
/// auth reliably works on `POST` (the akadmin token is bootstrapped well
/// before gunicorn even starts, but early requests against a just-booted
/// worker can still come back `403 unauthenticated`, auth backends
/// presumably still warming up on that worker). The real controller has
/// no problem outliving this — `error_policy` just retries — but this
/// test's own `wait_for_authentik_id` timeout is too short to survive a
/// failed-then-retried first attempt, so wait here for what the test
/// actually needs: an authenticated API call to actually succeed.
fn authentik_auth_is_ready() -> bool {
    let out = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "3",
            "-H",
            &format!("Authorization: Bearer {BOOTSTRAP_TOKEN}"),
            &format!("{AUTHENTIK_BASE}/api/v3/core/groups/"),
        ])
        .output();
    matches!(out, Ok(o) if o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim().starts_with('2'))
}

fn wait_for_authentik_ready(deadline: Instant) {
    while !authentik_is_ready() {
        assert!(
            Instant::now() < deadline,
            "authentik-server did not become ready ({AUTHENTIK_BASE}/-/health/ready/) in time \
             — check /tmp/ak.log inside the authentik-server/authentik-worker containers"
        );
        std::thread::sleep(Duration::from_secs(2));
    }
    while !authentik_auth_is_ready() {
        assert!(
            Instant::now() < deadline,
            "authentik-server reported healthy but bearer-token auth against \
             {AUTHENTIK_BASE}/api/v3/core/groups/ never started working in time \
             — check /tmp/ak.log inside the authentik-server container"
        );
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Every CRD here shares `#[kube(status = "AuthentikStatus")]` — this just
/// gives the round-trip helpers below a uniform way to read
/// `status.authentikId` across kinds without duplicating them per-CRD.
trait HasAuthentikStatus {
    fn authentik_status(&self) -> Option<&AuthentikStatus>;
}

impl HasAuthentikStatus for AuthentikGroup {
    fn authentik_status(&self) -> Option<&AuthentikStatus> {
        self.status.as_ref()
    }
}

impl HasAuthentikStatus for AuthentikUser {
    fn authentik_status(&self) -> Option<&AuthentikStatus> {
        self.status.as_ref()
    }
}

impl HasAuthentikStatus for AuthentikOutpost {
    fn authentik_status(&self) -> Option<&AuthentikStatus> {
        self.status.as_ref()
    }
}

impl HasAuthentikStatus for AuthentikBrand {
    fn authentik_status(&self) -> Option<&AuthentikStatus> {
        self.status.as_ref()
    }
}

async fn wait_for_authentik_id<K>(api: &Api<K>, name: &str) -> String
where
    K: kube::Resource + Clone + std::fmt::Debug + serde::de::DeserializeOwned + HasAuthentikStatus,
    K::DynamicType: Default,
{
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let obj = api.get(name).await.expect("CR must be gettable");
            if let Some(id) = obj
                .authentik_status()
                .and_then(|s| s.authentik_id.as_deref())
            {
                return id.to_string();
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| {
        panic!("controller must sync status.authentikId against the real sidecar Authentik within the timeout")
    })
}

/// Shared shape for every `*_controller_round_trips_against_real_sidecar_authentik`
/// test: create the CR, wait for the controller to sync a real Authentik-side
/// pk onto `status.authentikId`, delete it and confirm the finalizer actually
/// removed the Authentik-side object (not just the CR) rather than just
/// returning a 201 that happened to deserialize, then recreate under the
/// same name — a second create only succeeds, and only gets a *different*
/// pk, if the first was genuinely gone server-side (otherwise Authentik
/// rejects the name collision and `status.authentikId` never gets set).
async fn assert_round_trips_against_real_authentik<K>(
    api: &Api<K>,
    name: &str,
    make: impl Fn() -> K,
) where
    K: kube::Resource
        + Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + serde::Serialize
        + HasAuthentikStatus,
    K::DynamicType: Default,
{
    api.create(&PostParams::default(), &make())
        .await
        .expect("CR create must succeed");
    let first_id = wait_for_authentik_id(api, name).await;

    api.delete(name, &Default::default())
        .await
        .expect("CR delete must succeed");
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if api.get(name).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    })
    .await
    .expect(
        "finalizer cleanup must remove the CR (and the real Authentik object) within the timeout",
    );

    api.create(&PostParams::default(), &make()).await.expect(
        "recreating the same Authentik-side name must succeed once the first is truly gone",
    );
    let second_id = wait_for_authentik_id(api, name).await;
    assert_ne!(
        first_id, second_id,
        "recreated object must get a fresh pk from real Authentik, not reuse the deleted one"
    );

    api.delete(name, &Default::default()).await.ok();
}

fn new_ctx(client: kube::Client) -> Arc<Ctx> {
    let gateway = AuthentikHttpGateway::new(format!("{AUTHENTIK_BASE}/api/v3"), BOOTSTRAP_TOKEN);
    let gateway_factory = Arc::new(StaticGatewayFactory::new(Arc::new(gateway)));
    let secrets = Arc::new(K8sSecretStore::new(client.clone()));
    Arc::new(Ctx {
        client,
        gateway_factory,
        secrets,
    })
}

#[tokio::test]
#[ignore = "needs this repo's Che devworkspace sidecars (devfile.yaml); run via `task test:live-authentik`"]
async fn group_controller_round_trips_against_real_sidecar_authentik() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let pod = workspace_pod_name();
    ensure_ak_running(&pod, "authentik-server", "server");
    ensure_ak_running(&pod, "authentik-worker", "worker");
    wait_for_authentik_ready(Instant::now() + READY_TIMEOUT);

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();
    let ctx = new_ctx(client.clone());

    tokio::spawn(controller::group::run(client.clone(), ctx));

    let groups: Api<AuthentikGroup> = Api::all(client.clone());
    let name = format!("weebo-sidecar-live-test-group-{}", std::process::id());

    let make_group = || AuthentikGroup {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            ..Default::default()
        },
        spec: AuthentikGroupSpec {
            name: name.clone(),
            is_superuser: false,
            parent_ref: None,
            attributes: Default::default(),
        },
        status: None,
    };

    assert_round_trips_against_real_authentik(&groups, &name, make_group).await;
}

#[tokio::test]
#[ignore = "needs this repo's Che devworkspace sidecars (devfile.yaml); run via `task test:live-authentik`"]
async fn user_controller_round_trips_against_real_sidecar_authentik() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let pod = workspace_pod_name();
    ensure_ak_running(&pod, "authentik-server", "server");
    ensure_ak_running(&pod, "authentik-worker", "worker");
    wait_for_authentik_ready(Instant::now() + READY_TIMEOUT);

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();
    let ctx = new_ctx(client.clone());

    tokio::spawn(controller::user::run(client.clone(), ctx));

    let users: Api<AuthentikUser> = Api::all(client.clone());
    let name = format!("weebo-sidecar-live-test-user-{}", std::process::id());

    let make_user = || AuthentikUser {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            ..Default::default()
        },
        spec: AuthentikUserSpec {
            username: name.clone(),
            name: name.clone(),
            email: format!("{name}@weebo.local"),
            is_active: true,
            group_refs: Default::default(),
        },
        status: None,
    };

    assert_round_trips_against_real_authentik(&users, &name, make_user).await;
}

#[tokio::test]
#[ignore = "needs this repo's Che devworkspace sidecars (devfile.yaml); run via `task test:live-authentik`"]
async fn outpost_controller_round_trips_against_real_sidecar_authentik() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let pod = workspace_pod_name();
    ensure_ak_running(&pod, "authentik-server", "server");
    ensure_ak_running(&pod, "authentik-worker", "worker");
    wait_for_authentik_ready(Instant::now() + READY_TIMEOUT);

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();
    let ctx = new_ctx(client.clone());

    tokio::spawn(controller::outpost::run(client.clone(), ctx));

    let outposts: Api<AuthentikOutpost> = Api::all(client.clone());
    let name = format!("weebo-sidecar-live-test-outpost-{}", std::process::id());

    let make_outpost = || AuthentikOutpost {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            ..Default::default()
        },
        spec: AuthentikOutpostSpec {
            name: name.clone(),
            r#type: OutpostType::Proxy,
            config: serde_json::json!({}),
        },
        status: None,
    };

    assert_round_trips_against_real_authentik(&outposts, &name, make_outpost).await;
}

#[tokio::test]
#[ignore = "needs this repo's Che devworkspace sidecars (devfile.yaml); run via `task test:live-authentik`"]
async fn brand_controller_round_trips_against_real_sidecar_authentik() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let pod = workspace_pod_name();
    ensure_ak_running(&pod, "authentik-server", "server");
    ensure_ak_running(&pod, "authentik-worker", "worker");
    wait_for_authentik_ready(Instant::now() + READY_TIMEOUT);

    let cluster = EnvTestCluster::start().await;
    let client = cluster.client();
    let ctx = new_ctx(client.clone());

    tokio::spawn(controller::brand::run(client.clone(), ctx));

    let brands: Api<AuthentikBrand> = Api::all(client.clone());
    let name = format!("weebo-sidecar-live-test-brand-{}", std::process::id());

    // Deliberately `default: false` — the default-election path is
    // already covered by `domain::brand_election`'s unit tests and by
    // `adapters-inbound/tests/brand_controller.rs`'s envtest+wiremock
    // tests; this test's only job is proving a real Authentik round trip
    // for the brand CRUD itself.
    let make_brand = || AuthentikBrand {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            ..Default::default()
        },
        spec: AuthentikBrandSpec {
            domain: format!("{name}.example.com"),
            default: false,
            branding_title: None,
            branding_logo: None,
            branding_favicon: None,
            branding_default_flow_background: None,
            default_application_ref: None,
            flow_authentication: None,
            flow_invalidation: None,
            flow_recovery: None,
            flow_unenrollment: None,
            flow_user_settings: None,
        },
        status: None,
    };

    assert_round_trips_against_real_authentik(&brands, &name, make_brand).await;
}
