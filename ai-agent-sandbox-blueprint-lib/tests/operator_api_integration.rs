//! Tier 2: Operator API + Real Sidecar integration tests.
//!
//! These tests boot a real Docker sidecar AND the operator API server,
//! then exercise the full lifecycle: sandbox listing, provision tracking,
//! session auth (EIP-191 + PASETO), command execution, and cleanup.
//!
//! Run:
//!   REAL_SIDECAR=1 cargo test --test operator_api_integration -- --test-threads=1
//!
//! Requires Docker and a local sidecar image (default: tangle-sidecar:local).

use std::collections::HashMap;
use std::net::TcpListener;
use std::time::Duration;

use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions,
};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use sandbox_runtime::operator_api::operator_api_router;
use sandbox_runtime::provision_progress::{self, ProvisionPhase};
use sandbox_runtime::session_auth;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const AUTH_TOKEN: &str = "test-operator-api-token-3b8c7d";
const CONTAINER_NAME: &str = "test-operator-api-sidecar";
const CONTAINER_PORT: u16 = 8080;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn should_run() -> bool {
    std::env::var("REAL_SIDECAR")
        .map(|v| v == "1")
        .unwrap_or(false)
}

macro_rules! skip_unless_real {
    () => {
        if !should_run() {
            eprintln!("Skipped (set REAL_SIDECAR=1 to enable)");
            return;
        }
    };
}

struct TestSidecar {
    url: String,
    #[allow(dead_code)]
    container_id: String,
}

struct TestOperatorApi {
    url: String,
    #[allow(dead_code)]
    handle: std::thread::JoinHandle<()>,
}

static SIDECAR: OnceCell<TestSidecar> = OnceCell::const_new();
static OPERATOR_API: OnceCell<TestOperatorApi> = OnceCell::const_new();

async fn docker_builder() -> DockerBuilder {
    match DockerBuilder::new().await {
        Ok(b) => b,
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_default();
            let mac_sock = format!("unix://{home}/.docker/run/docker.sock");
            DockerBuilder::with_address(&mac_sock)
                .await
                .expect("Docker daemon not reachable")
        }
    }
}

async fn ensure_sidecar() -> &'static TestSidecar {
    SIDECAR
        .get_or_init(|| async {
            if std::env::var("REQUEST_TIMEOUT_SECS").is_err() {
                unsafe { std::env::set_var("REQUEST_TIMEOUT_SECS", "300") };
            }

            let image = std::env::var("SIDECAR_IMAGE")
                .unwrap_or_else(|_| "tangle-sidecar:local".to_string());

            let builder = docker_builder().await;

            // Clean up leftover container from a previous crashed run.
            let _ = builder
                .client()
                .remove_container(
                    CONTAINER_NAME,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;

            let mut port_bindings = PortMap::new();
            port_bindings.insert(
                format!("{CONTAINER_PORT}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: None,
                }]),
            );

            let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
            exposed_ports.insert(format!("{CONTAINER_PORT}/tcp"), HashMap::new());

            let env_vars = vec![
                format!("SIDECAR_PORT={CONTAINER_PORT}"),
                format!("SIDECAR_AUTH_TOKEN={AUTH_TOKEN}"),
                "NODE_ENV=development".to_string(),
                "PORT_WATCHER_ENABLED=false".to_string(),
            ];

            let override_config = BollardConfig {
                exposed_ports: Some(exposed_ports),
                host_config: Some(HostConfig {
                    port_bindings: Some(port_bindings),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let mut container = Container::new(builder.client(), image.clone())
                .with_name(CONTAINER_NAME.to_string())
                .env(env_vars)
                .config_override(override_config);

            container
                .start(false)
                .await
                .unwrap_or_else(|e| panic!("Failed to start sidecar ({image}): {e}"));

            let container_id = container
                .id()
                .expect("Container has no ID after start")
                .to_string();

            let inspect = builder
                .client()
                .inspect_container(&container_id, None::<InspectContainerOptions>)
                .await
                .expect("Failed to inspect container");

            let host_port = inspect
                .network_settings
                .as_ref()
                .and_then(|ns| ns.ports.as_ref())
                .and_then(|p| p.get(&format!("{CONTAINER_PORT}/tcp")))
                .and_then(|v| v.as_ref())
                .and_then(|v| v.first())
                .and_then(|b| b.host_port.as_ref())
                .and_then(|p| p.parse::<u16>().ok())
                .expect("Could not extract host port");

            let url = format!("http://127.0.0.1:{host_port}");

            // Wait for healthy.
            let client = Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                if tokio::time::Instant::now() > deadline {
                    panic!("Sidecar not healthy within 60s at {url}");
                }
                match client.get(format!("{url}/health")).send().await {
                    Ok(resp) if resp.status().is_success() => break,
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            TestSidecar { url, container_id }
        })
        .await
}

async fn ensure_operator_api() -> &'static TestOperatorApi {
    OPERATOR_API
        .get_or_init(|| async {
            // Find a random available port.
            let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind random port");
            let port = listener.local_addr().unwrap().port();
            drop(listener);

            // Spawn the API server on a dedicated OS thread with its own tokio
            // runtime so it survives across #[tokio::test] boundaries (each test
            // creates its own runtime that is dropped when the test finishes).
            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let app = operator_api_router();
                    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
                        .await
                        .expect("Failed to bind operator API port");
                    axum::serve(listener, app).await.ok();
                });
            });

            let url = format!("http://127.0.0.1:{port}");

            // Wait for the server to be ready.
            let client = Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                if tokio::time::Instant::now() > deadline {
                    panic!("Operator API not ready within 5s at {url}");
                }
                match client.get(format!("{url}/api/provisions")).send().await {
                    Ok(resp) if resp.status().is_success() => break,
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            TestOperatorApi { url, handle }
        })
        .await
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn sidecar_auth() -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {AUTH_TOKEN}")).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full lifecycle: provision tracking → sandbox listing → auth flow → command exec → cleanup.
#[tokio::test]
async fn full_operator_api_lifecycle() {
    skip_unless_real!();

    // Set up a fresh state dir so sandbox store doesn't conflict with other tests.
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", tmp.path()) };

    let sidecar = ensure_sidecar().await;
    let api = ensure_operator_api().await;

    // -----------------------------------------------------------------------
    // Step 1: Verify sandboxes list is initially empty (fresh state dir).
    // -----------------------------------------------------------------------
    let resp = http()
        .get(format!("{}/api/sandboxes", api.url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["sandboxes"].as_array().unwrap().is_empty(),
        "Sandboxes should be empty initially: {body}"
    );
    eprintln!("Step 1 OK: sandbox list is empty");

    // -----------------------------------------------------------------------
    // Step 2: Start and track provision progress.
    // -----------------------------------------------------------------------
    let call_id: u64 = 12345;
    provision_progress::start_provision(call_id).unwrap();

    // Verify queued state via API.
    let resp = http()
        .get(format!("{}/api/provisions/{call_id}", api.url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["phase"], "queued");
    assert_eq!(body["progress_pct"], 0);
    eprintln!("Step 2a OK: provision started as queued");

    // Walk through phases.
    for (phase, expected_phase_str, expected_pct) in [
        (ProvisionPhase::ImagePull, "image_pull", 20),
        (ProvisionPhase::ContainerCreate, "container_create", 40),
        (ProvisionPhase::ContainerStart, "container_start", 60),
        (ProvisionPhase::HealthCheck, "health_check", 80),
    ] {
        provision_progress::update_provision(call_id, phase, None, None).unwrap();

        let resp = http()
            .get(format!("{}/api/provisions/{call_id}", api.url))
            .send()
            .await
            .unwrap();
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["phase"], expected_phase_str, "phase mismatch");
        assert_eq!(body["progress_pct"], expected_pct, "progress_pct mismatch");
    }

    // Mark as ready with a sandbox ID.
    provision_progress::update_provision(
        call_id,
        ProvisionPhase::Ready,
        Some("Sandbox ready".into()),
        Some("sandbox-test-123".into()),
    )
    .unwrap();

    let resp = http()
        .get(format!("{}/api/provisions/{call_id}", api.url))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["phase"], "ready");
    assert_eq!(body["progress_pct"], 100);
    assert_eq!(body["sandbox_id"], "sandbox-test-123");
    eprintln!("Step 2b OK: provision walked through all phases to Ready");

    // Verify it shows in the provisions list.
    let resp = http()
        .get(format!("{}/api/provisions", api.url))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let provisions = body["provisions"].as_array().unwrap();
    assert!(
        provisions.iter().any(|p| p["call_id"] == call_id),
        "Provision {call_id} not found in list: {body}"
    );
    eprintln!("Step 2c OK: provision visible in list");

    // -----------------------------------------------------------------------
    // Step 3: Session auth flow (challenge → sign → token → validate).
    // -----------------------------------------------------------------------
    use k256::ecdsa::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    // Derive expected address.
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let address_hash = keccak256(pubkey_uncompressed);
    let expected_address = format!("0x{}", hex::encode(&address_hash[12..]));

    // 3a: Request challenge via operator API.
    let resp = http()
        .post(format!("{}/api/auth/challenge", api.url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let challenge: Value = resp.json().await.unwrap();
    let nonce = challenge["nonce"].as_str().unwrap();
    let message = challenge["message"].as_str().unwrap();
    assert_eq!(nonce.len(), 64, "nonce should be 32 bytes hex");
    assert!(
        message.contains(nonce),
        "message should contain nonce: {message}"
    );
    eprintln!("Step 3a OK: got challenge nonce={}", &nonce[..16]);

    // 3b: Sign the message with EIP-191.
    let prefixed = format!(
        "\x19Ethereum Signed Message:\n{}{}",
        message.len(),
        message
    );
    let digest = keccak256(prefixed.as_bytes());
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .expect("signing failed");
    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&signature.to_bytes());
    sig_bytes.push(recovery_id.to_byte() + 27);
    let sig_hex = format!("0x{}", hex::encode(&sig_bytes));

    // 3c: Exchange signature for PASETO token via operator API.
    let resp = http()
        .post(format!("{}/api/auth/session", api.url))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "nonce": nonce,
            "signature": sig_hex,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "session exchange should succeed");
    let session: Value = resp.json().await.unwrap();
    let token = session["token"].as_str().unwrap();
    let address = session["address"].as_str().unwrap();
    assert!(token.starts_with("v4.local."), "should be a PASETO v4 token");
    assert_eq!(
        address, expected_address,
        "recovered address should match signing key"
    );
    eprintln!(
        "Step 3c OK: got PASETO token for address {address}, token={}...",
        &token[..30]
    );

    // 3d: Validate token via server-side function.
    let claims = session_auth::validate_session_token(token).unwrap();
    assert_eq!(claims.address, expected_address);
    eprintln!("Step 3d OK: token validates correctly");

    // 3e: Verify extract_session_from_headers works.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {token}").parse().unwrap(),
    );
    let extracted =
        sandbox_runtime::operator_api::extract_session_from_headers(&headers).unwrap();
    assert_eq!(extracted.address, expected_address);
    eprintln!("Step 3e OK: extract_session_from_headers works");

    // -----------------------------------------------------------------------
    // Step 4: Execute a command on the real sidecar.
    // -----------------------------------------------------------------------
    let resp = http()
        .post(format!("{}/terminals/commands", sidecar.url))
        .header(AUTHORIZATION, sidecar_auth())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo operator-api-test-ok", "timeout": 10000}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "exec status: {}", resp.status());
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("operator-api-test-ok"),
        "stdout: '{stdout}'"
    );
    eprintln!("Step 4 OK: sidecar command executed successfully");

    // -----------------------------------------------------------------------
    // Step 5: Verify CORS headers.
    // -----------------------------------------------------------------------
    let resp = http()
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/api/sandboxes", api.url),
        )
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "GET")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().contains_key("access-control-allow-origin"),
        "CORS headers should be present"
    );
    eprintln!("Step 5 OK: CORS preflight works");

    eprintln!("\n=== All operator API integration tests passed ===");
}

/// Verify that invalid auth flows are rejected.
#[tokio::test]
async fn auth_error_cases() {
    skip_unless_real!();

    let _sidecar = ensure_sidecar().await;
    let api = ensure_operator_api().await;

    // Bad signature should return 401.
    let resp = http()
        .post(format!("{}/api/auth/challenge", api.url))
        .send()
        .await
        .unwrap();
    let challenge: Value = resp.json().await.unwrap();
    let nonce = challenge["nonce"].as_str().unwrap();

    let resp = http()
        .post(format!("{}/api/auth/session", api.url))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "nonce": nonce,
            "signature": "0xdeadbeef",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "bad signature should be rejected");
    eprintln!("Auth error case: bad signature → 401 OK");

    // Replaying the same nonce should fail (already consumed).
    let resp = http()
        .post(format!("{}/api/auth/session", api.url))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "nonce": nonce,
            "signature": "0xdeadbeef",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "replayed nonce should be rejected");
    eprintln!("Auth error case: replayed nonce → 401 OK");

    // Nonexistent provision should be 404.
    let resp = http()
        .get(format!("{}/api/provisions/999999999", api.url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    eprintln!("Auth error case: nonexistent provision → 404 OK");
}

/// Verify failed provision tracking.
#[tokio::test]
async fn provision_failure_tracking() {
    skip_unless_real!();

    let _sidecar = ensure_sidecar().await;
    let api = ensure_operator_api().await;

    let call_id: u64 = 99999;
    provision_progress::start_provision(call_id).unwrap();

    provision_progress::update_provision(
        call_id,
        ProvisionPhase::ImagePull,
        Some("Pulling image".into()),
        None,
    )
    .unwrap();

    provision_progress::update_provision(
        call_id,
        ProvisionPhase::Failed,
        Some("Docker pull failed: image not found".into()),
        None,
    )
    .unwrap();

    let resp = http()
        .get(format!("{}/api/provisions/{call_id}", api.url))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["phase"], "failed");
    assert_eq!(body["progress_pct"], 0);
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("image not found"),
        "message: {body}"
    );
    eprintln!("Provision failure tracking OK");
}

// ---------------------------------------------------------------------------
// Keccak256 helper (same as session_auth uses internally)
// ---------------------------------------------------------------------------

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn zz_cleanup_container() {
    if !should_run() {
        return;
    }

    let builder = docker_builder().await;
    let _ = builder
        .client()
        .remove_container(
            CONTAINER_NAME,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    eprintln!("Cleaned up container: {CONTAINER_NAME}");
}
