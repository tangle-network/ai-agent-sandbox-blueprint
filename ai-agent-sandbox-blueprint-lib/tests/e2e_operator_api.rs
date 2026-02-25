//! End-to-end test: Sandbox blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path:
//!   1. Provision sandbox via Tangle job (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise every operator API endpoint against the real sidecar
//!   5. Clean up via Tangle job
//!
//! Run:
//!   SIDECAR_E2E=1 cargo test -p ai-agent-sandbox-blueprint-lib \
//!       --test e2e_operator_api -- --test-threads=1
//!
//! Requires:
//!   - Docker (for sidecar containers)
//!   - tangle-sidecar:local image (or set SIDECAR_IMAGE)
//!   - TNT core artifacts (run scripts/fetch-localtestnet-fixtures.sh)

use ai_agent_sandbox_blueprint_lib::{
    JOB_SANDBOX_CREATE, JOB_SANDBOX_DELETE, SandboxCreateOutput, SandboxCreateRequest,
    SandboxIdRequest, JsonResponse, router,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::sol_types::SolValue;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);
const JOB_RESULT_TIMEOUT: Duration = Duration::from_secs(180);

/// The key the harness uses to submit jobs. The Caller extractor sees this address,
/// making it the sandbox owner.
const SERVICE_OWNER_KEY_HEX: &str =
    "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

macro_rules! step {
    ($n:expr, $msg:expr) => {
        eprintln!("[Step {: >2}] {}", $n, $msg);
    };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

fn setup_sidecar_env() {
    let image =
        std::env::var("SIDECAR_IMAGE").unwrap_or_else(|_| "tangle-sidecar:local".to_string());
    unsafe {
        std::env::set_var("SIDECAR_IMAGE", &image);
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "60");
        std::env::set_var("SESSION_AUTH_SECRET", "e2e-test-secret-key");
    }
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// Derive the EVM address for a private key hex string.
fn address_from_key(key_hex: &str) -> String {
    use k256::ecdsa::SigningKey;
    let key_bytes = hex::decode(key_hex).unwrap();
    let signing_key = SigningKey::from_bytes((&key_bytes[..]).into()).unwrap();
    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let hash = keccak256(pubkey_uncompressed);
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Full auth flow: challenge → EIP-191 sign → PASETO token.
async fn get_auth_token(api_url: &str, key_hex: &str) -> Result<(String, String)> {
    use k256::ecdsa::SigningKey;

    let key_bytes = hex::decode(key_hex).unwrap();
    let signing_key = SigningKey::from_bytes((&key_bytes[..]).into()).unwrap();

    // Step 1: Get challenge
    let challenge_resp = http()
        .post(format!("{api_url}/api/auth/challenge"))
        .send()
        .await?;
    assert_eq!(challenge_resp.status(), 200, "challenge should succeed");
    let challenge: Value = challenge_resp.json().await?;
    let nonce = challenge["nonce"].as_str().context("nonce")?;
    let message = challenge["message"].as_str().context("message")?;

    // Step 2: EIP-191 sign
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
    let digest = keccak256(prefixed.as_bytes());
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .expect("signing failed");
    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&signature.to_bytes());
    sig_bytes.push(recovery_id.to_byte() + 27);
    let sig_hex = format!("0x{}", hex::encode(&sig_bytes));

    // Step 3: Exchange for session token
    let session_resp = http()
        .post(format!("{api_url}/api/auth/session"))
        .header("content-type", "application/json")
        .json(&json!({ "nonce": nonce, "signature": sig_hex }))
        .send()
        .await?;
    assert_eq!(session_resp.status(), 200, "session exchange should succeed");
    let session: Value = session_resp.json().await?;
    let token = session["token"].as_str().context("token")?.to_string();
    let address = session["address"].as_str().context("address")?.to_string();

    assert!(token.starts_with("v4.local."), "should be PASETO v4 token");
    Ok((token, address))
}

/// Wait for the operator API to respond.
async fn wait_for_api(url: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("Operator API not ready within 10s at {url}");
        }
        if let Ok(r) = http().get(format!("{url}/health")).send().await {
            if r.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait for sidecar to become healthy.
async fn wait_for_sidecar(url: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("Sidecar not healthy within 90s at {url}");
        }
        if let Ok(resp) = http().get(format!("{url}/health")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn spawn_harness() -> Result<Option<BlueprintHarness>> {
    match BlueprintHarness::builder(router())
        .poll_interval(Duration::from_millis(50))
        .spawn()
        .await
    {
        Ok(harness) => Ok(Some(harness)),
        Err(err) => {
            if missing_tnt_core_artifacts(&err) {
                eprintln!("Skipping: {err}");
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sandbox_full_lifecycle_via_operator_api() -> Result<()> {
    let guard = HARNESS_LOCK.lock().await;
    let result = timeout(ANVIL_TEST_TIMEOUT, async {
        if std::env::var("SIDECAR_E2E").ok().as_deref() != Some("1") {
            eprintln!("Skipped (set SIDECAR_E2E=1 to enable)");
            return Ok(());
        }

        let _ = tracing_subscriber::fmt::try_init();
        setup_sidecar_env();

        // ─── Step 1: Spawn harness ───────────────────────────────────────
        step!(1, "Spawning BlueprintHarness (Anvil + Runner)...");
        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };

        let owner_address = address_from_key(SERVICE_OWNER_KEY_HEX);
        eprintln!("  Owner address: {owner_address}");

        // ─── Step 2: Provision sandbox via Tangle ────────────────────────
        step!(2, "Submitting JOB_SANDBOX_CREATE via Tangle...");
        let create_payload = SandboxCreateRequest {
            name: "e2e-sandbox".to_string(),
            image: "agent-dev".to_string(),
            stack: "default".to_string(),
            agent_identifier: "default-agent".to_string(),
            env_json: "{}".to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 3600,
            idle_timeout_seconds: 900,
            cpu_cores: 2,
            memory_mb: 4096,
            disk_gb: 20,
            tee_required: false,
            tee_type: 0,
        }
        .abi_encode();

        let create_sub = harness
            .submit_job(JOB_SANDBOX_CREATE, Bytes::from(create_payload))
            .await?;
        let create_output = harness
            .wait_for_job_result_with_deadline(create_sub, JOB_RESULT_TIMEOUT)
            .await?;
        let create_receipt = SandboxCreateOutput::abi_decode(&create_output)?;
        let create_json: Value = serde_json::from_str(&create_receipt.json)?;
        let sandbox_id = create_receipt.sandboxId.clone();
        let sidecar_url = create_json["sidecarUrl"]
            .as_str()
            .context("missing sidecarUrl")?
            .to_string();
        eprintln!("  Sandbox: id={sandbox_id}, url={sidecar_url}");

        // ─── Step 3: Boot operator API ───────────────────────────────────
        step!(3, "Starting operator API server...");
        let api_app = sandbox_runtime::operator_api::operator_api_router();
        let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let api_port = api_listener.local_addr()?.port();
        let _api_handle = tokio::spawn(async move {
            axum::serve(
                api_listener,
                api_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .ok();
        });
        let api_url = format!("http://127.0.0.1:{api_port}");
        wait_for_api(&api_url).await?;
        eprintln!("  API ready at {api_url}");

        // ─── Step 4: Wait for sidecar health ─────────────────────────────
        step!(4, "Waiting for sidecar to become healthy...");
        wait_for_sidecar(&sidecar_url).await?;
        eprintln!("  Sidecar healthy at {sidecar_url}");

        // ─── Step 5: Auth flow ───────────────────────────────────────────
        step!(5, "Authenticating with operator API...");
        let (token, authed_address) = get_auth_token(&api_url, SERVICE_OWNER_KEY_HEX).await?;
        eprintln!("  Authenticated as {authed_address}");

        let auth = format!("Bearer {token}");

        // ─── Step 6: List sandboxes ──────────────────────────────────────
        step!(6, "Listing sandboxes...");
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await?;
        let sandboxes = body["sandboxes"].as_array().context("sandboxes array")?;
        assert!(
            sandboxes.iter().any(|s| s["id"] == sandbox_id),
            "sandbox {sandbox_id} should be in list: {body}"
        );
        let sb = sandboxes.iter().find(|s| s["id"] == sandbox_id).unwrap();
        assert_eq!(sb["state"], "running");
        eprintln!("  Found sandbox in list, state=running");

        // ─── Step 7: Exec ────────────────────────────────────────────────
        step!(7, "Executing command via operator API...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "echo e2e-sandbox-test-ok"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "exec should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["exit_code"], 0, "exit code should be 0");
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-sandbox-test-ok"),
            "stdout should contain test string: {body}"
        );
        eprintln!("  Exec OK: stdout={}", body["stdout"]);

        // ─── Step 8: SSH provision ───────────────────────────────────────
        step!(8, "Provisioning SSH key...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-test";
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "SSH provision should succeed: {:?}", resp.text().await);

        // Re-send to verify idempotency
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "SSH provision should be idempotent");
        eprintln!("  SSH key provisioned (+ idempotency verified)");

        // ─── Step 9: SSH revoke ──────────────────────────────────────────
        step!(9, "Revoking SSH key...");
        let resp = http()
            .delete(format!("{api_url}/api/sandboxes/{sandbox_id}/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "SSH revoke should succeed");
        eprintln!("  SSH key revoked");

        // ─── Step 10: Secrets inject ─────────────────────────────────────
        step!(10, "Injecting secrets...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/secrets"))
            .header("authorization", &auth)
            .json(&json!({"env_json": {"E2E_SECRET": "test-value-42"}}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "secrets inject should succeed: {:?}", resp.text().await);
        eprintln!("  Secrets injected");

        // Wait for sidecar to restart after secrets injection (container recreation)
        tokio::time::sleep(Duration::from_secs(3)).await;
        wait_for_sidecar(&sidecar_url).await?;

        // ─── Step 11: Verify secrets via exec ────────────────────────────
        step!(11, "Verifying secrets via exec...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "printenv E2E_SECRET"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await?;
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("test-value-42"),
            "E2E_SECRET should be visible in env: {body}"
        );
        eprintln!("  Secret verified in container env");

        // ─── Step 12: Secrets wipe ───────────────────────────────────────
        step!(12, "Wiping secrets...");
        let resp = http()
            .delete(format!("{api_url}/api/sandboxes/{sandbox_id}/secrets"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "secrets wipe should succeed");
        eprintln!("  Secrets wiped");

        // Wait for sidecar restart
        tokio::time::sleep(Duration::from_secs(3)).await;
        wait_for_sidecar(&sidecar_url).await?;

        // ─── Step 13: Verify wipe via exec ───────────────────────────────
        step!(13, "Verifying secrets wiped...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "printenv E2E_SECRET || echo 'NOT_SET'"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await?;
        let stdout = body["stdout"].as_str().unwrap_or("");
        assert!(
            !stdout.contains("test-value-42"),
            "E2E_SECRET should be gone after wipe: {body}"
        );
        eprintln!("  Secret confirmed wiped");

        // ─── Step 14: Stop sandbox ───────────────────────────────────────
        step!(14, "Stopping sandbox...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/stop"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "stop should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Sandbox stopped");

        // ─── Step 15: List shows stopped ─────────────────────────────────
        step!(15, "Verifying stopped state in list...");
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .header("authorization", &auth)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("sandbox should still be in list while stopped")?;
        assert_eq!(sb["state"], "stopped");
        eprintln!("  List confirms stopped state");

        // ─── Step 16: Resume sandbox ─────────────────────────────────────
        step!(16, "Resuming sandbox...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/resume"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "resume should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Sandbox resumed");

        // Wait for sidecar to be healthy again
        wait_for_sidecar(&sidecar_url).await?;

        // ─── Step 17: Exec after resume ──────────────────────────────────
        step!(17, "Exec after resume...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "echo resumed-ok"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await?;
        assert!(body["stdout"].as_str().unwrap_or("").contains("resumed-ok"));
        eprintln!("  Exec after resume OK");

        // ─── Step 18: Input validation ───────────────────────────────────
        step!(18, "Testing input validation...");

        // Empty command → 400
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": ""}))
            .send()
            .await?;
        assert_eq!(resp.status(), 400, "empty command should be rejected");

        // Bad SSH key format → 400
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": "not-a-real-key"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 400, "invalid SSH key should be rejected");
        eprintln!("  Input validation OK");

        // ─── Step 19: Auth rejection ─────────────────────────────────────
        step!(19, "Testing auth rejection...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/exec"))
            .json(&json!({"command": "echo should-fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth should return 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 20: Delete via Tangle ──────────────────────────────────
        step!(20, "Deleting sandbox via Tangle job...");
        let delete_payload = SandboxIdRequest {
            sandbox_id: sandbox_id.clone(),
        }
        .abi_encode();
        let delete_sub = harness
            .submit_job(JOB_SANDBOX_DELETE, Bytes::from(delete_payload))
            .await?;
        let delete_output = harness
            .wait_for_job_result_with_deadline(delete_sub, JOB_RESULT_TIMEOUT)
            .await?;
        let delete_receipt = JsonResponse::abi_decode(&delete_output)?;
        let delete_json: Value = serde_json::from_str(&delete_receipt.json)?;
        assert_eq!(delete_json["deleted"], true);
        eprintln!("  Sandbox deleted");

        // ─── Step 21: Verify gone ────────────────────────────────────────
        step!(21, "Verifying sandbox is gone...");
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .header("authorization", &auth)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let sandboxes = body["sandboxes"].as_array().context("sandboxes array")?;
        assert!(
            !sandboxes.iter().any(|s| s["id"] == sandbox_id),
            "deleted sandbox should not appear in list"
        );
        eprintln!("  Sandbox confirmed gone from list");

        // ─── Step 22: Shutdown ───────────────────────────────────────────
        step!(22, "Shutting down harness...");
        harness.shutdown().await;

        eprintln!("\n=== All sandbox E2E operator API tests passed ===");
        Ok(())
    })
    .await;

    drop(guard);
    result.context("sandbox_full_lifecycle_via_operator_api timed out")?
}
