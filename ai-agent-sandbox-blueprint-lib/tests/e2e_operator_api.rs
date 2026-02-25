//! End-to-end test: Sandbox blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path:
//!   1. Provision sandbox via Tangle job (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise every operator API endpoint against the real sidecar
//!   5. Verify cross-owner tenant isolation
//!   6. Exercise workflow create / cancel on-chain
//!   7. Clean up via Tangle job
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
    JOB_SANDBOX_CREATE, JOB_SANDBOX_DELETE, JOB_WORKFLOW_CANCEL, JOB_WORKFLOW_CREATE,
    JsonResponse, SandboxCreateOutput, SandboxCreateRequest, SandboxIdRequest,
    WorkflowControlRequest, WorkflowCreateRequest, router,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::sol_types::SolValue;
use once_cell::sync::Lazy;
use sandbox_runtime::e2e_step;
use sandbox_runtime::test_utils::*;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);
const JOB_RESULT_TIMEOUT: Duration = Duration::from_secs(180);

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

async fn spawn_harness() -> Result<Option<BlueprintHarness>> {
    match BlueprintHarness::builder(router())
        .poll_interval(Duration::from_millis(50))
        .spawn()
        .await
    {
        Ok(h) => Ok(Some(h)),
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

// ─────────────────────────────────────────────────────────────────────────────
// Test: Full sandbox lifecycle with on-chain verification
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sandbox_full_lifecycle() -> Result<()> {
    let _guard = HARNESS_LOCK.lock().await;
    timeout(ANVIL_TEST_TIMEOUT, async {
        if std::env::var("SIDECAR_E2E").ok().as_deref() != Some("1") {
            eprintln!("Skipped (set SIDECAR_E2E=1 to enable)");
            return Ok(());
        }

        let _ = tracing_subscriber::fmt::try_init();
        setup_sidecar_env();

        let owner_address = address_from_key(OWNER_KEY);

        // ─── Step 1: Spawn harness ───────────────────────────────────────
        e2e_step!(1, "Spawning BlueprintHarness (Anvil + Runner)...");
        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };
        eprintln!("  Owner address: {owner_address}");

        // ─── Step 2: Provision sandbox via Tangle ────────────────────────
        e2e_step!(2, "Submitting JOB_SANDBOX_CREATE via Tangle...");
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

        // Verify on-chain result is valid ABI
        let create_receipt = SandboxCreateOutput::abi_decode(&create_output)
            .context("failed to ABI-decode SandboxCreateOutput from on-chain result")?;
        let create_json: Value = serde_json::from_str(&create_receipt.json)
            .context("create result JSON is malformed")?;
        let sandbox_id = create_receipt.sandboxId.clone();
        let initial_sidecar_url = create_json["sidecarUrl"]
            .as_str()
            .context("missing sidecarUrl in create result")?
            .to_string();

        assert!(!sandbox_id.is_empty(), "sandbox_id should not be empty");
        assert!(
            initial_sidecar_url.starts_with("http"),
            "sidecar_url should be HTTP: {initial_sidecar_url}"
        );
        eprintln!("  Created: id={sandbox_id}, url={initial_sidecar_url}");

        // ─── Step 3: Boot operator API ───────────────────────────────────
        e2e_step!(3, "Starting operator API server...");
        let (api_url, _api_handle) = spawn_operator_api().await?;
        eprintln!("  API ready at {api_url}");

        // ─── Step 4: Wait for sidecar health ─────────────────────────────
        e2e_step!(4, "Waiting for sidecar to become healthy...");
        wait_for_sidecar(&initial_sidecar_url).await?;
        eprintln!("  Sidecar healthy");

        // ─── Step 5: Auth as owner ───────────────────────────────────────
        e2e_step!(5, "Authenticating as owner...");
        let (token, authed_address) = get_auth_token(&api_url, OWNER_KEY).await?;
        assert_eq!(
            authed_address.to_lowercase(),
            owner_address.to_lowercase(),
            "auth should return the owner address"
        );
        let auth = format!("Bearer {token}");
        eprintln!("  Authenticated as {authed_address}");

        // ─── Step 6: List sandboxes ──────────────────────────────────────
        e2e_step!(6, "Listing sandboxes...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sandboxes = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        let sb = sandboxes
            .iter()
            .find(|s| s["id"] == sandbox_id)
            .context("sandbox should be in list")?;
        assert_eq!(sb["state"], "running");
        eprintln!("  Found sandbox, state=running");

        // ─── Step 7: Exec ────────────────────────────────────────────────
        e2e_step!(7, "Executing command...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo e2e-sandbox-test-ok"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 0);
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-sandbox-test-ok"),
            "stdout should contain test string: {body}"
        );
        eprintln!("  Exec OK");

        // ─── Step 8: SSH provision + idempotency ─────────────────────────
        e2e_step!(8, "SSH provision + idempotency...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-test";
        let ssh_body = json!({"username": "agent", "public_key": ssh_key});
        let path = format!("/api/sandboxes/{sandbox_id}/ssh");
        assert_api_status(&api_url, "POST", &path, &auth, ssh_body.clone(), 200).await;
        // Idempotent second call
        assert_api_status(&api_url, "POST", &path, &auth, ssh_body, 200).await;
        eprintln!("  SSH provisioned (idempotent)");

        // ─── Step 9: SSH revoke ──────────────────────────────────────────
        e2e_step!(9, "SSH revoke...");
        assert_api_status(
            &api_url,
            "DELETE",
            &path,
            &auth,
            json!({"username": "agent", "public_key": ssh_key}),
            200,
        )
        .await;
        eprintln!("  SSH revoked");

        // ─── Step 10: Stop sandbox ───────────────────────────────────────
        e2e_step!(10, "Stopping sandbox...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/stop"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Stopped");

        // ─── Step 11: Verify stopped in list ─────────────────────────────
        e2e_step!(11, "Verifying stopped state...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("sandbox should still be in list")?;
        assert_eq!(sb["state"], "stopped");
        eprintln!("  Confirmed stopped");

        // ─── Step 12: Resume sandbox ─────────────────────────────────────
        e2e_step!(12, "Resuming sandbox...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/resume"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Resumed");

        // ─── Step 13: Re-read sidecar URL (Docker assigns new ports) ─────
        e2e_step!(13, "Re-reading sidecar URL after resume...");
        let resumed_url = get_sidecar_url(&api_url, &auth, &sandbox_id).await?;
        eprintln!("  Post-resume URL: {resumed_url}");
        if resumed_url != initial_sidecar_url {
            eprintln!(
                "  Port changed: {} → {} (expected after Docker restart)",
                initial_sidecar_url, resumed_url
            );
        }
        wait_for_sidecar(&resumed_url).await?;

        // ─── Step 14: Exec after resume (using UPDATED URL) ──────────────
        e2e_step!(14, "Exec after resume...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo resumed-ok"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 0);
        assert!(body["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("resumed-ok"));
        eprintln!("  Exec after resume OK");

        // ─── Step 15: Secrets inject ─────────────────────────────────────
        e2e_step!(15, "Injecting secrets...");
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &auth,
            json!({"env_json": {"E2E_SECRET": "test-value-42"}}),
            200,
        )
        .await;
        eprintln!("  Secrets injected");

        // Wait for container recreation
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Re-read sidecar URL (secrets recreation may change port)
        let secrets_url = get_sidecar_url(&api_url, &auth, &sandbox_id).await
            .unwrap_or(resumed_url.clone());
        wait_for_sidecar(&secrets_url).await?;

        // ─── Step 16: Verify secret visible ──────────────────────────────
        e2e_step!(16, "Verifying secret via exec...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "printenv E2E_SECRET"}),
        )
        .await?;
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("test-value-42"),
            "secret should be in env: {body}"
        );
        eprintln!("  Secret verified");

        // ─── Step 17: Secrets wipe ───────────────────────────────────────
        e2e_step!(17, "Wiping secrets...");
        let resp = http()
            .delete(format!("{api_url}/api/sandboxes/{sandbox_id}/secrets"))
            .header("authorization", &auth)
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        eprintln!("  Secrets wiped");

        tokio::time::sleep(Duration::from_secs(3)).await;
        let wiped_url = get_sidecar_url(&api_url, &auth, &sandbox_id).await
            .unwrap_or(secrets_url);
        wait_for_sidecar(&wiped_url).await?;

        // ─── Step 18: Verify secret wiped ────────────────────────────────
        e2e_step!(18, "Verifying secret wiped...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "printenv E2E_SECRET || echo NOT_SET"}),
        )
        .await?;
        assert!(
            !body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("test-value-42"),
            "secret should be gone: {body}"
        );
        eprintln!("  Secret confirmed wiped");

        // ─── Step 19: Input validation ───────────────────────────────────
        e2e_step!(19, "Testing input validation...");
        // Empty command → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": ""}),
            400,
        )
        .await;
        // Invalid SSH key → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/ssh"),
            &auth,
            json!({"username": "agent", "public_key": "not-a-real-key"}),
            400,
        )
        .await;
        eprintln!("  Input validation OK");

        // ─── Step 20: Auth rejection ─────────────────────────────────────
        e2e_step!(20, "Testing auth rejection...");
        let resp = http()
            .post(format!(
                "{api_url}/api/sandboxes/{sandbox_id}/exec"
            ))
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth → 401");

        let resp = http()
            .post(format!(
                "{api_url}/api/sandboxes/{sandbox_id}/exec"
            ))
            .header("authorization", "Bearer v4.local.invalid-token-garbage")
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "bad PASETO → 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 21: Cross-owner isolation ──────────────────────────────
        e2e_step!(21, "Testing cross-owner tenant isolation...");
        let non_owner_address = address_from_key(NON_OWNER_KEY);
        let (non_owner_token, addr) = get_auth_token(&api_url, NON_OWNER_KEY).await?;
        assert_eq!(
            addr.to_lowercase(),
            non_owner_address.to_lowercase(),
            "non-owner auth should return their own address"
        );
        let non_owner_auth = format!("Bearer {non_owner_token}");

        // Non-owner should see empty sandbox list (filtered by owner)
        let body = api_get(&api_url, "/api/sandboxes", &non_owner_auth).await?;
        let non_owner_sandboxes = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            non_owner_sandboxes.is_empty(),
            "non-owner should see zero sandboxes, got: {body}"
        );
        eprintln!("  Non-owner sees empty list (isolation confirmed)");

        // ─── Step 22: Delete sandbox via Tangle ──────────────────────────
        e2e_step!(22, "Deleting sandbox via Tangle...");
        let delete_payload = SandboxIdRequest {
            sandbox_id: sandbox_id.clone(),
        }
        .abi_encode();
        let delete_sub = harness
            .submit_job(JOB_SANDBOX_DELETE, Bytes::from(delete_payload))
            .await?;
        let delete_output = harness
            .wait_for_job_result_with_deadline(delete_sub, JOB_RESULT_TIMEOUT)
            .await
            .context("delete job result not received")?;

        let delete_receipt = JsonResponse::abi_decode(&delete_output)
            .context("failed to ABI-decode delete result")?;
        let delete_json: Value = serde_json::from_str(&delete_receipt.json)?;
        assert_eq!(delete_json["deleted"], true, "delete response: {delete_json}");
        eprintln!("  Deleted: {delete_json}");

        // ─── Step 23: Verify sandbox gone ────────────────────────────────
        e2e_step!(23, "Verifying sandbox gone from list...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let remaining = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            !remaining.iter().any(|s| s["id"] == sandbox_id),
            "deleted sandbox should not appear"
        );
        eprintln!("  Confirmed gone");

        // ─── Step 24: Shutdown ───────────────────────────────────────────
        e2e_step!(24, "Shutting down...");
        harness.shutdown().await;
        eprintln!("\n=== All sandbox E2E tests passed (24 steps) ===");
        Ok(())
    })
    .await
    .context("sandbox_full_lifecycle timed out")?
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: Workflow create / cancel through Tangle
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn workflow_create_and_cancel() -> Result<()> {
    let _guard = HARNESS_LOCK.lock().await;
    timeout(ANVIL_TEST_TIMEOUT, async {
        if std::env::var("SIDECAR_E2E").ok().as_deref() != Some("1") {
            eprintln!("Skipped (set SIDECAR_E2E=1 to enable)");
            return Ok(());
        }

        let _ = tracing_subscriber::fmt::try_init();
        setup_sidecar_env();

        // ─── Step 1: Spawn harness ───────────────────────────────────────
        e2e_step!(1, "Spawning BlueprintHarness for workflow test...");
        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };

        // ─── Step 2: Create workflow via Tangle ──────────────────────────
        e2e_step!(2, "Submitting JOB_WORKFLOW_CREATE...");
        let create_payload = WorkflowCreateRequest {
            name: "e2e-test-workflow".to_string(),
            workflow_json: serde_json::to_string(&json!({
                "sidecar_url": "http://placeholder:8080",
                "prompt": "test workflow prompt",
                "max_turns": 1,
            }))?,
            trigger_type: "manual".to_string(),
            trigger_config: String::new(),
            sandbox_config_json: "{}".to_string(),
        }
        .abi_encode();

        let create_sub = harness
            .submit_job(JOB_WORKFLOW_CREATE, Bytes::from(create_payload))
            .await?;
        let create_output = harness
            .wait_for_job_result_with_deadline(create_sub, JOB_RESULT_TIMEOUT)
            .await
            .context("workflow create result not received")?;

        let create_result = JsonResponse::abi_decode(&create_output)
            .context("failed to decode workflow create result")?;
        let create_json: Value = serde_json::from_str(&create_result.json)?;
        assert_eq!(create_json["status"], "active", "workflow should be active: {create_json}");
        let workflow_id = create_json["workflowId"]
            .as_u64()
            .context("missing workflowId")?;
        eprintln!("  Workflow created: id={workflow_id}, status=active");

        // ─── Step 3: Cancel workflow via Tangle ──────────────────────────
        e2e_step!(3, "Submitting JOB_WORKFLOW_CANCEL...");
        let cancel_payload = WorkflowControlRequest {
            workflow_id,
        }
        .abi_encode();

        let cancel_sub = harness
            .submit_job(JOB_WORKFLOW_CANCEL, Bytes::from(cancel_payload))
            .await?;
        let cancel_output = harness
            .wait_for_job_result_with_deadline(cancel_sub, JOB_RESULT_TIMEOUT)
            .await
            .context("workflow cancel result not received")?;

        let cancel_result = JsonResponse::abi_decode(&cancel_output)
            .context("failed to decode workflow cancel result")?;
        let cancel_json: Value = serde_json::from_str(&cancel_result.json)?;
        assert_eq!(cancel_json["status"], "canceled", "workflow should be canceled: {cancel_json}");
        eprintln!("  Workflow canceled: {cancel_json}");

        // ─── Step 4: Shutdown ────────────────────────────────────────────
        e2e_step!(4, "Shutting down...");
        harness.shutdown().await;
        eprintln!("\n=== Workflow E2E tests passed (4 steps) ===");
        Ok(())
    })
    .await
    .context("workflow_create_and_cancel timed out")?
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP assertion helpers (used only in this test file)
// ─────────────────────────────────────────────────────────────────────────────

async fn api_get(api_url: &str, path: &str, auth: &str) -> Result<Value> {
    let resp = http()
        .get(format!("{api_url}{path}"))
        .header("authorization", auth)
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "GET {path} returned {status}: {body}"
    );
    Ok(body)
}

async fn api_post(api_url: &str, path: &str, auth: &str, body: Value) -> Result<Value> {
    let resp = http()
        .post(format!("{api_url}{path}"))
        .header("authorization", auth)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let resp_body: Value = resp.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "POST {path} returned {status}: {resp_body}"
    );
    Ok(resp_body)
}

async fn assert_api_status(
    api_url: &str,
    method: &str,
    path: &str,
    auth: &str,
    body: Value,
    expected_status: u16,
) {
    let url = format!("{api_url}{path}");
    let resp = match method {
        "POST" => http()
            .post(&url)
            .header("authorization", auth)
            .json(&body)
            .send()
            .await,
        "DELETE" => http()
            .delete(&url)
            .header("authorization", auth)
            .json(&body)
            .send()
            .await,
        _ => panic!("unsupported method: {method}"),
    };
    let resp = resp.unwrap_or_else(|e| panic!("{method} {path} failed: {e}"));
    assert_eq!(
        resp.status().as_u16(),
        expected_status,
        "{method} {path} expected {expected_status}, got {}",
        resp.status()
    );
}
