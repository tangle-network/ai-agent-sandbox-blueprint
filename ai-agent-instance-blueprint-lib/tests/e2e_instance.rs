//! End-to-end test: Instance blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path for the instance blueprint:
//!   1. Provision instance via Tangle job (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise every instance operator API endpoint against the real sidecar
//!   5. Deprovision via Tangle job
//!
//! Run:
//!   SIDECAR_E2E=1 cargo test -p ai-agent-instance-blueprint-lib \
//!       --test e2e_instance -- --test-threads=1
//!
//! Requires:
//!   - Docker (for sidecar containers)
//!   - tangle-sidecar:local image (or set SIDECAR_IMAGE)
//!   - TNT core artifacts (run scripts/fetch-localtestnet-fixtures.sh)

use ai_agent_instance_blueprint_lib::{
    JOB_DEPROVISION, JOB_PROVISION, JsonResponse, ProvisionOutput, ProvisionRequest, router,
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
/// making it the instance owner.
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
        std::env::set_var("SESSION_AUTH_SECRET", "e2e-instance-test-secret-key");
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
async fn instance_full_lifecycle_via_operator_api() -> Result<()> {
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

        // ─── Step 2: Provision instance via Tangle ───────────────────────
        step!(2, "Submitting JOB_PROVISION via Tangle...");
        let provision_payload = ProvisionRequest {
            name: "e2e-instance".to_string(),
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
            sidecar_token: String::new(),
            tee_required: false,
            tee_type: 0,
        }
        .abi_encode();

        let provision_sub = harness
            .submit_job(JOB_PROVISION, Bytes::from(provision_payload))
            .await?;
        let provision_output = harness
            .wait_for_job_result_with_deadline(provision_sub, JOB_RESULT_TIMEOUT)
            .await?;
        let provision_receipt = ProvisionOutput::abi_decode(&provision_output)?;
        let sandbox_id = provision_receipt.sandbox_id.clone();
        let sidecar_url = provision_receipt.sidecar_url.clone();
        eprintln!("  Instance: id={sandbox_id}, url={sidecar_url}");

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
        step!(6, "Listing sandboxes (should include instance)...");
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
            "instance sandbox {sandbox_id} should be in list: {body}"
        );
        let sb = sandboxes.iter().find(|s| s["id"] == sandbox_id).unwrap();
        assert_eq!(sb["state"], "running");
        eprintln!("  Found instance sandbox in list, state=running");

        // ─── Step 7: Instance exec (singleton endpoint) ──────────────────
        step!(7, "Executing command via instance operator API...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "echo e2e-instance-test-ok"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "instance exec should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["exit_code"], 0, "exit code should be 0");
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-instance-test-ok"),
            "stdout should contain test string: {body}"
        );
        eprintln!("  Instance exec OK: stdout={}", body["stdout"]);

        // ─── Step 8: Instance SSH provision ──────────────────────────────
        step!(8, "Provisioning SSH key via instance endpoint...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-instance-test";
        let resp = http()
            .post(format!("{api_url}/api/sandbox/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "SSH provision should succeed: {:?}", resp.text().await);
        eprintln!("  SSH key provisioned via instance endpoint");

        // ─── Step 9: Instance SSH revoke ─────────────────────────────────
        step!(9, "Revoking SSH key via instance endpoint...");
        let resp = http()
            .delete(format!("{api_url}/api/sandbox/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "SSH revoke should succeed");
        eprintln!("  SSH key revoked via instance endpoint");

        // ─── Step 10: Instance stop ──────────────────────────────────────
        step!(10, "Stopping instance via operator API...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/stop"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "stop should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Instance stopped");

        // ─── Step 11: List shows stopped ─────────────────────────────────
        step!(11, "Verifying stopped state in list...");
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .header("authorization", &auth)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("instance sandbox should still be in list while stopped")?;
        assert_eq!(sb["state"], "stopped");
        eprintln!("  List confirms stopped state");

        // ─── Step 12: Instance resume ────────────────────────────────────
        step!(12, "Resuming instance via operator API...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/resume"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "resume should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Instance resumed");

        // Wait for sidecar to be healthy again
        wait_for_sidecar(&sidecar_url).await?;

        // ─── Step 13: Exec after resume ──────────────────────────────────
        step!(13, "Exec after resume via instance endpoint...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "echo instance-resumed-ok"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await?;
        assert!(body["stdout"].as_str().unwrap_or("").contains("instance-resumed-ok"));
        eprintln!("  Exec after resume OK");

        // ─── Step 14: Input validation ───────────────────────────────────
        step!(14, "Testing input validation on instance endpoints...");

        // Empty command → 400
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": ""}))
            .send()
            .await?;
        assert_eq!(resp.status(), 400, "empty command should be rejected");

        // Bad SSH key format → 400
        let resp = http()
            .post(format!("{api_url}/api/sandbox/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "agent", "public_key": "not-a-real-key"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 400, "invalid SSH key should be rejected");
        eprintln!("  Input validation OK");

        // ─── Step 15: Auth rejection ─────────────────────────────────────
        step!(15, "Testing auth rejection on instance endpoints...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .json(&json!({"command": "echo should-fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth should return 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 16: Deprovision via Tangle ─────────────────────────────
        step!(16, "Deprovisioning instance via Tangle job...");
        let deprovision_payload = JsonResponse {
            json: json!({"action": "deprovision"}).to_string(),
        }
        .abi_encode();
        let deprovision_sub = harness
            .submit_job(JOB_DEPROVISION, Bytes::from(deprovision_payload))
            .await?;
        let deprovision_output = harness
            .wait_for_job_result_with_deadline(deprovision_sub, JOB_RESULT_TIMEOUT)
            .await?;
        let deprovision_receipt = JsonResponse::abi_decode(&deprovision_output)?;
        let deprovision_json: Value = serde_json::from_str(&deprovision_receipt.json)?;
        assert_eq!(deprovision_json["deprovisioned"], true);
        eprintln!("  Instance deprovisioned: {deprovision_json}");

        // ─── Step 17: Verify gone ────────────────────────────────────────
        step!(17, "Verifying instance sandbox is gone...");
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .header("authorization", &auth)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let sandboxes = body["sandboxes"].as_array().context("sandboxes array")?;
        assert!(
            !sandboxes.iter().any(|s| s["id"] == sandbox_id),
            "deprovisioned instance should not appear in list"
        );
        eprintln!("  Instance sandbox confirmed gone from list");

        // ─── Step 18: Shutdown ───────────────────────────────────────────
        step!(18, "Shutting down harness...");
        harness.shutdown().await;

        eprintln!("\n=== All instance E2E operator API tests passed ===");
        Ok(())
    })
    .await;

    drop(guard);
    result.context("instance_full_lifecycle_via_operator_api timed out")?
}
