//! Functional smoke test for leader election (`crates/operator/src/main.rs`,
//! `kube-leader-election`): spawns two real compiled `operator` processes
//! against the same (envtest) cluster and confirms exactly one of them
//! becomes — and is recorded as — the leader on the shared `Lease`, while
//! both keep serving the webhook regardless of who leads.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use k8s_openapi::api::coordination::v1::Lease;
use kube::Api;
use testkit::envtest::EnvTestCluster;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_port(port: u16, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "operator did not start listening on :{port} in time"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "operator process exited early"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn assert_tls_handshake_ok(port: u16) {
    let mut curl = Command::new("curl")
        .args([
            "-k",
            "-s",
            "--max-time",
            "5",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
            &format!("https://127.0.0.1:{port}/validate"),
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("curl must be available");
    curl.stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({
                "apiVersion": "admission.k8s.io/v1",
                "kind": "AdmissionReview",
                "request": {
                    "uid": "11111111-1111-1111-1111-111111111111",
                    "namespace": "default",
                    "kind": { "kind": "AuthentikApplication" }
                }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = curl.wait_with_output().unwrap();
    assert!(output.status.success(), "curl against :{port} must succeed");
}

#[tokio::test]
async fn exactly_one_replica_becomes_leader() {
    let cluster = EnvTestCluster::start().await;

    let dir = std::env::temp_dir().join(format!("leader-election-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let kubeconfig_path = dir.join("kubeconfig.yaml");
    std::fs::write(&kubeconfig_path, cluster.kubeconfig_str()).unwrap();

    let cert_path = dir.join("tls.crt");
    let key_path = dir.join("tls.key");
    let openssl = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
        ])
        .status()
        .expect("openssl must be available to generate a throwaway cert");
    assert!(openssl.success(), "openssl cert generation must succeed");

    let lease_name = "leader-election-test-lease";
    let spawn_replica = |hostname: &str, port: u16| {
        KillOnDrop(
            Command::new(env!("CARGO_BIN_EXE_operator"))
                .env("KUBECONFIG", &kubeconfig_path)
                .env("WEBHOOK_TLS_CERT_FILE", &cert_path)
                .env("WEBHOOK_TLS_KEY_FILE", &key_path)
                .env("WEBHOOK_PORT", port.to_string())
                .env("POD_NAMESPACE", "default")
                .env("HOSTNAME", hostname)
                .env("LEASE_NAME", lease_name)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("operator binary must spawn"),
        )
    };

    let mut replica_a = spawn_replica("replica-a", 18443);
    let mut replica_b = spawn_replica("replica-b", 18444);

    // Both webhooks come up immediately, independent of leader election.
    wait_for_port(18443, &mut replica_a.0);
    wait_for_port(18444, &mut replica_b.0);
    assert_tls_handshake_ok(18443);
    assert_tls_handshake_ok(18444);

    // Leader election settles quickly (first `try_acquire_or_renew` wins,
    // no pre-existing Lease) — poll rather than a fixed sleep.
    let leases: Api<Lease> = Api::namespaced(cluster.client(), "default");
    let deadline = Instant::now() + Duration::from_secs(15);
    let holder = loop {
        if let Ok(lease) = leases.get(lease_name).await
            && let Some(holder) = lease.spec.and_then(|s| s.holder_identity)
        {
            break holder;
        }
        assert!(
            Instant::now() < deadline,
            "no leader was elected onto the Lease in time"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    assert!(
        holder == "replica-a" || holder == "replica-b",
        "Lease holderIdentity {holder:?} must be one of the two spawned replicas"
    );

    std::fs::remove_dir_all(&dir).ok();
}
