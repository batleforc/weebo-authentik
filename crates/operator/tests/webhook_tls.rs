//! Functional smoke test for the webhook's TLS wiring (`crates/operator/src/main.rs`):
//! spawns the real compiled `operator` binary against a real (envtest)
//! control plane and a throwaway self-signed cert, then confirms
//! `https://127.0.0.1:8443/validate` actually completes a TLS handshake
//! and returns a real `AdmissionReview` JSON response. No existing test
//! exercised the operator's HTTP/TLS transport layer at all before this.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use testkit::envtest::EnvTestCluster;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn webhook_serves_real_tls_and_responds() {
    let cluster = EnvTestCluster::start().await;

    let dir = std::env::temp_dir().join(format!("webhook-tls-test-{}", std::process::id()));
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

    let mut child = KillOnDrop(
        Command::new(env!("CARGO_BIN_EXE_operator"))
            .env("KUBECONFIG", &kubeconfig_path)
            .env("WEBHOOK_TLS_CERT_FILE", &cert_path)
            .env("WEBHOOK_TLS_KEY_FILE", &key_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("operator binary must spawn"),
    );

    // Poll for the port to accept connections rather than a fixed sleep
    // — startup time (kube client init, TLS cert load) isn't fixed.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if std::net::TcpStream::connect("127.0.0.1:8443").is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "operator did not start listening on :8443 in time"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "operator process exited early"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    let mut curl = Command::new("curl")
        .args([
            "-k",
            "-s",
            "--max-time",
            "5",
            "-X",
            "POST",
            "https://127.0.0.1:8443/validate",
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
                    "kind": {
                        "group": "authentik.weebo.io", "version": "v1alpha1",
                        "kind": "AuthentikApplication",
                    },
                    // A real apiserver always sends resource/operation/
                    // userInfo too — the webhook's typed AdmissionRequest
                    // (kube::core::admission) requires them, unlike the
                    // pre-typed-refactor handler which only read uid/
                    // namespace/kind.kind and defaulted the rest.
                    "resource": {
                        "group": "authentik.weebo.io", "version": "v1alpha1",
                        "resource": "authentikapplications",
                    },
                    "operation": "CREATE",
                    "userInfo": {},
                }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = curl.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "curl must complete successfully (proves the TLS handshake worked)"
    );

    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("response must be valid JSON");
    assert_eq!(body["kind"], "AdmissionReview");
    assert!(body["response"]["uid"].is_string());
    // The envtest cluster has no AuthentikNamespacePolicy at all, so
    // default-deny must apply — this is the fail-closed decision the
    // webhook exists to enforce, not just its TLS transport.
    assert_eq!(body["response"]["allowed"], false);

    std::fs::remove_dir_all(&dir).ok();
}
