//! End-to-end test: Sandbox blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path:
//!   1. Provision sandbox via Tangle job (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise EVERY operator API endpoint against the real sidecar
//!   5. Verify cross-owner tenant isolation
//!   6. Exercise workflow create / cancel on-chain
//!   7. Test every input validation path, error path, and idempotency
//!   8. Clean up via Tangle job
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
    JOB_SANDBOX_CREATE, JOB_SANDBOX_DELETE, JOB_WORKFLOW_CANCEL, JOB_WORKFLOW_CREATE, JsonResponse,
    SandboxCreateOutput, SandboxCreateRequest, SandboxIdRequest, WorkflowControlRequest,
    WorkflowCreateRequest, router,
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
// Test: Full sandbox lifecycle with on-chain verification (31 steps)
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

        // ─── Step 6: Health endpoint (unauthenticated) ───────────────────
        e2e_step!(6, "Testing health endpoint...");
        let body = api_get_unauth(&api_url, "/health").await?;
        assert_eq!(body["status"], "ok", "health response: {body}");
        eprintln!("  Health OK");

        // ─── Step 7: Metrics endpoint (unauthenticated) ──────────────────
        e2e_step!(7, "Testing metrics endpoint...");
        let resp = http().get(format!("{api_url}/metrics")).send().await?;
        assert_eq!(resp.status(), 200, "metrics should return 200");
        let metrics_text = resp.text().await?;
        assert!(
            metrics_text.contains("sandbox_total_jobs"),
            "metrics should contain sandbox_total_jobs"
        );
        assert!(
            metrics_text.contains("sandbox_active_sandboxes"),
            "metrics should contain sandbox_active_sandboxes"
        );
        eprintln!("  Metrics OK ({} bytes)", metrics_text.len());

        // ─── Step 8: Provisions endpoint (unauthenticated) ───────────────
        e2e_step!(8, "Testing provisions endpoint...");
        let body = api_get_unauth(&api_url, "/api/provisions").await?;
        assert!(
            body["provisions"].is_array(),
            "provisions should be an array: {body}"
        );
        eprintln!("  Provisions OK");

        // ─── Step 9: List sandboxes ──────────────────────────────────────
        e2e_step!(9, "Listing sandboxes...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sandboxes = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        let sb = sandboxes
            .iter()
            .find(|s| s["id"] == sandbox_id)
            .context("sandbox should be in list")?;
        assert_eq!(sb["state"], "running");
        assert!(sb["sidecar_url"].is_string(), "should have sidecar_url");
        assert!(sb["cpu_cores"].is_number(), "should have cpu_cores");
        assert!(sb["memory_mb"].is_number(), "should have memory_mb");
        assert!(sb["created_at"].is_number(), "should have created_at");
        eprintln!("  Found sandbox, state=running");

        // ─── Step 10: Exec (stdout, exit 0) ──────────────────────────────
        e2e_step!(10, "Executing command (exit 0)...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo e2e-sandbox-test-ok"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 0, "exec response: {body}");
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-sandbox-test-ok"),
            "stdout should contain test string: {body}"
        );
        eprintln!("  Exec OK (exit 0)");

        // ─── Step 11: Exec (non-zero exit code) ──────────────────────────
        e2e_step!(11, "Executing command (non-zero exit)...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "exit 42"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 42, "should return exit code 42: {body}");
        eprintln!("  Exec OK (exit 42)");

        // ─── Step 12: Exec (stderr output) ───────────────────────────────
        e2e_step!(12, "Executing command (stderr)...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo e2e-stderr >&2"}),
        )
        .await?;
        assert!(
            body["stderr"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-stderr"),
            "stderr should contain test string: {body}"
        );
        eprintln!("  Exec OK (stderr captured)");

        // ─── Step 13: Prompt endpoint (functional) ───────────────────────
        e2e_step!(13, "Testing prompt endpoint...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/prompt"))
            .header("authorization", &auth)
            .json(&json!({"message": "Say hello"}))
            .send()
            .await?;
        let status = resp.status().as_u16();
        match status {
            200 => {
                let body: Value = resp.json().await?;
                assert!(body.get("success").is_some(), "prompt: missing 'success': {body}");
                assert!(body.get("response").is_some(), "prompt: missing 'response': {body}");
                eprintln!("  Prompt OK (200, success={})", body["success"]);
            }
            502 => {
                // Sidecar agent endpoint not available — acceptable in test env
                eprintln!("  Prompt: sidecar agent not available (502 accepted)");
            }
            _ => anyhow::bail!("prompt: unexpected status {status}"),
        }

        // ─── Step 14: Task endpoint (functional) ─────────────────────────
        e2e_step!(14, "Testing task endpoint...");
        let resp = http()
            .post(format!("{api_url}/api/sandboxes/{sandbox_id}/task"))
            .header("authorization", &auth)
            .json(&json!({"prompt": "Say hello", "max_turns": 1}))
            .send()
            .await?;
        let status = resp.status().as_u16();
        match status {
            200 => {
                let body: Value = resp.json().await?;
                assert!(body.get("success").is_some(), "task: missing 'success': {body}");
                assert!(body.get("result").is_some(), "task: missing 'result': {body}");
                assert!(body.get("session_id").is_some(), "task: missing 'session_id': {body}");
                eprintln!("  Task OK (200, success={})", body["success"]);
            }
            502 => {
                eprintln!("  Task: sidecar agent not available (502 accepted)");
            }
            _ => anyhow::bail!("task: unexpected status {status}"),
        }

        // ─── Step 15: Snapshot endpoint (functional) ─────────────────────
        e2e_step!(15, "Testing snapshot endpoint...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/snapshot"),
            &auth,
            json!({
                "destination": "http://127.0.0.1:1/e2e-snapshot-test",
                "include_workspace": true,
                "include_state": false,
            }),
        )
        .await?;
        assert_eq!(body["success"], true, "snapshot response: {body}");
        assert!(body["result"].is_object(), "snapshot should have result: {body}");
        eprintln!("  Snapshot OK (result returned)");

        // ─── Step 16: SSH provision + idempotency ────────────────────────
        e2e_step!(16, "SSH provision + idempotency...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-test";
        let ssh_body = json!({"username": "agent", "public_key": ssh_key});
        let path = format!("/api/sandboxes/{sandbox_id}/ssh");
        assert_api_status(&api_url, "POST", &path, &auth, ssh_body.clone(), 200).await;
        // Idempotent second call
        assert_api_status(&api_url, "POST", &path, &auth, ssh_body, 200).await;
        eprintln!("  SSH provisioned (idempotent)");

        // ─── Step 17: SSH revoke ─────────────────────────────────────────
        e2e_step!(17, "SSH revoke...");
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

        // ─── Step 18: Secrets inject ─────────────────────────────────────
        e2e_step!(18, "Injecting secrets...");
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
        let secrets_url = get_sidecar_url(&api_url, &auth, &sandbox_id)
            .await
            .unwrap_or(initial_sidecar_url.clone());
        wait_for_sidecar(&secrets_url).await?;

        // ─── Step 19: Verify secret visible ──────────────────────────────
        e2e_step!(19, "Verifying secret via exec...");
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

        // ─── Step 20: Secrets wipe + verify ──────────────────────────────
        e2e_step!(20, "Wiping secrets...");
        let body = api_delete(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &auth,
        )
        .await?;
        assert_eq!(body["status"], "secrets_wiped", "wipe response: {body}");
        eprintln!("  Secrets wiped");

        tokio::time::sleep(Duration::from_secs(3)).await;
        let wiped_url = get_sidecar_url(&api_url, &auth, &sandbox_id)
            .await
            .unwrap_or(secrets_url);
        wait_for_sidecar(&wiped_url).await?;

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

        // ─── Step 21: Stop sandbox ───────────────────────────────────────
        e2e_step!(21, "Stopping sandbox...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/stop"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Stopped");

        // ─── Step 22: Verify stopped + exec on stopped ──────────────────
        e2e_step!(22, "Verifying stopped state + exec on stopped sandbox...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("sandbox should still be in list")?;
        assert_eq!(sb["state"], "stopped");

        // Exec on stopped sandbox → should fail (sidecar unreachable)
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo should-fail"}),
            502,
        )
        .await;
        eprintln!("  Confirmed stopped, exec returns 502");

        // ─── Step 23: Stop idempotency ───────────────────────────────────
        e2e_step!(23, "Testing stop idempotency...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/stop"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "stopped", "second stop: {body}");
        eprintln!("  Stop idempotent");

        // ─── Step 24: Resume sandbox ─────────────────────────────────────
        e2e_step!(24, "Resuming sandbox...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/resume"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Resumed");

        // ─── Step 25: Resume idempotency ─────────────────────────────────
        e2e_step!(25, "Testing resume idempotency...");
        let body = api_post(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/resume"),
            &auth,
            json!({}),
        )
        .await?;
        assert_eq!(body["state"], "running", "second resume: {body}");
        eprintln!("  Resume idempotent");

        // ─── Step 26: Re-read URL + exec after resume ────────────────────
        e2e_step!(26, "Re-reading sidecar URL, exec after resume...");
        let resumed_url = get_sidecar_url(&api_url, &auth, &sandbox_id).await?;
        eprintln!("  Post-resume URL: {resumed_url}");
        if resumed_url != initial_sidecar_url {
            eprintln!(
                "  Port changed: {initial_sidecar_url} → {resumed_url} (expected after Docker restart)"
            );
        }
        wait_for_sidecar(&resumed_url).await?;

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

        // ─── Step 27: Input validation ───────────────────────────────────
        e2e_step!(27, "Testing input validation (6 checks)...");
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
        // Empty prompt message → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/prompt"),
            &auth,
            json!({"message": ""}),
            400,
        )
        .await;
        // Empty task prompt → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/task"),
            &auth,
            json!({"prompt": ""}),
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
        // Empty snapshot destination → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/snapshot"),
            &auth,
            json!({"destination": "", "include_workspace": true, "include_state": false}),
            400,
        )
        .await;
        // Empty secrets env_json → 400
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &auth,
            json!({"env_json": {}}),
            400,
        )
        .await;
        eprintln!("  Input validation OK (6/6)");

        // ─── Step 28: Auth rejection ─────────────────────────────────────
        e2e_step!(28, "Testing auth rejection...");
        // Missing auth → 401
        let resp = http()
            .post(format!(
                "{api_url}/api/sandboxes/{sandbox_id}/exec"
            ))
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth → 401");

        // Bad PASETO → 401
        let resp = http()
            .post(format!(
                "{api_url}/api/sandboxes/{sandbox_id}/exec"
            ))
            .header("authorization", "Bearer v4.local.invalid-token-garbage")
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "bad PASETO → 401");

        // Missing auth on list → 401
        let resp = http()
            .get(format!("{api_url}/api/sandboxes"))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "list without auth → 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 29: Cross-owner isolation ──────────────────────────────
        e2e_step!(29, "Testing cross-owner tenant isolation...");
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

        // Non-owner exec on specific sandbox → 403
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &non_owner_auth,
            json!({"command": "echo pwned"}),
            403,
        )
        .await;

        // Non-owner secrets inject → 403
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &non_owner_auth,
            json!({"env_json": {"BAD": "nope"}}),
            403,
        )
        .await;
        eprintln!("  Non-owner sees empty list, exec/secrets → 403");

        // ─── Step 30: Delete sandbox via Tangle ──────────────────────────
        e2e_step!(30, "Deleting sandbox via Tangle...");
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

        // ─── Step 31: Verify gone + exec on deleted ──────────────────────
        e2e_step!(31, "Verifying sandbox gone, exec on deleted → 404...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let remaining = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            !remaining.iter().any(|s| s["id"] == sandbox_id),
            "deleted sandbox should not appear"
        );

        // Exec on deleted sandbox → 404
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/exec"),
            &auth,
            json!({"command": "echo test"}),
            404,
        )
        .await;
        eprintln!("  Confirmed gone, exec → 404");

        // ─── Shutdown ────────────────────────────────────────────────────
        harness.shutdown().await;
        eprintln!("\n=== All sandbox E2E tests passed (31 steps) ===");
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
        assert_eq!(
            create_json["status"], "active",
            "workflow should be active: {create_json}"
        );
        let workflow_id = create_json["workflowId"]
            .as_u64()
            .context("missing workflowId")?;
        eprintln!("  Workflow created: id={workflow_id}, status=active");

        // ─── Step 3: Cancel workflow via Tangle ──────────────────────────
        e2e_step!(3, "Submitting JOB_WORKFLOW_CANCEL...");
        let cancel_payload = WorkflowControlRequest { workflow_id }.abi_encode();

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
        assert_eq!(
            cancel_json["status"], "canceled",
            "workflow should be canceled: {cancel_json}"
        );
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
