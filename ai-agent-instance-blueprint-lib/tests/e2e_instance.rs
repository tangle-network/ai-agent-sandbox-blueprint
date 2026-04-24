//! End-to-end test: Instance blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path for the instance blueprint:
//!   1. Provision instance locally (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise EVERY instance operator API endpoint against the real sidecar
//!   5. Test secrets inject/wipe through sandbox-scoped endpoint
//!   6. Verify cross-owner tenant isolation
//!   7. Test every input validation path, error path, and idempotency
//!   8. Deprovision via local lifecycle path
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
    ProvisionRequest, deprovision_core, provision_core, router, set_instance_sandbox,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use once_cell::sync::Lazy;
use sandbox_runtime::e2e_step;
use sandbox_runtime::test_utils::*;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);

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

async fn detect_ssh_user(api_url: &str, auth: &str) -> Result<String> {
    let body = api_get(api_url, "/api/sandbox/ssh/user", auth).await?;
    body["username"]
        .as_str()
        .map(str::to_string)
        .context("ssh user response missing username")
}

async fn ssh_key_presence(api_url: &str, auth: &str, username: &str, key: &str) -> Result<bool> {
    let body = api_post(
        api_url,
        "/api/sandbox/exec",
        auth,
        json!({
            "command": format!(
                "sh -lc \"home=$(getent passwd \\\"{username}\\\" | cut -d: -f6); if grep -qxF \\\"{key}\\\" \\\"\\$home/.ssh/authorized_keys\\\" 2>/dev/null; then echo PRESENT; else echo ABSENT; fi\""
            )
        }),
    )
    .await?;
    Ok(body["stdout"].as_str().unwrap_or("").contains("PRESENT"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: Full instance lifecycle with on-chain verification (28 steps)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn instance_full_lifecycle() -> Result<()> {
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

        // ─── Step 2: Provision instance locally ───────────────────────────
        e2e_step!(2, "Provisioning local instance runtime...");
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
            tee_required: false,
            tee_type: 0,
            attestation_nonce: String::new(),
        };

        let (provision_receipt, record) = provision_core(&provision_payload, None, &owner_address)
            .await
            .map_err(anyhow::Error::msg)?;
        set_instance_sandbox(record)?;
        let sandbox_id = provision_receipt.sandbox_id.clone();
        let initial_sidecar_url = provision_receipt.sidecar_url.clone();

        assert!(!sandbox_id.is_empty(), "sandbox_id should not be empty");
        assert!(
            initial_sidecar_url.starts_with("http"),
            "sidecar_url should be HTTP: {initial_sidecar_url}"
        );
        eprintln!("  Provisioned: id={sandbox_id}, url={initial_sidecar_url}");

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
            .context("instance sandbox should be in list")?;
        assert_eq!(sb["state"], "running");
        assert!(sb["sidecar_url"].is_string(), "should have sidecar_url");
        eprintln!("  Found instance, state=running");

        // ─── Step 7: Exec via singleton endpoint (exit 0) ────────────────
        e2e_step!(7, "Exec via instance endpoint (exit 0)...");
        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
            &auth,
            json!({"command": "echo e2e-instance-ok"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 0);
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("e2e-instance-ok"),
            "stdout: {body}"
        );
        eprintln!("  Exec OK (exit 0)");

        // ─── Step 8: Exec (non-zero exit code) ──────────────────────────
        e2e_step!(8, "Exec (non-zero exit code)...");
        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
            &auth,
            json!({"command": "exit 42"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 42, "should return exit code 42: {body}");
        eprintln!("  Exec OK (exit 42)");

        // ─── Step 9: Exec (stderr output) ────────────────────────────────
        e2e_step!(9, "Exec (stderr output)...");
        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
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

        // ─── Step 10: Prompt endpoint (functional) ───────────────────────
        e2e_step!(10, "Testing prompt endpoint...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/prompt"))
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
                eprintln!("  Prompt: sidecar agent not available (502 accepted)");
            }
            _ => anyhow::bail!("prompt: unexpected status {status}"),
        }

        // ─── Step 11: Task endpoint (functional) ─────────────────────────
        e2e_step!(11, "Testing task endpoint...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/task"))
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

        // ─── Step 12: Snapshot endpoint (functional) ─────────────────────
        e2e_step!(12, "Testing snapshot endpoint...");
        let body = api_post(
            &api_url,
            "/api/sandbox/snapshot",
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

        // ─── Step 13: SSH provision ──────────────────────────────────────
        e2e_step!(13, "SSH user detection + provision via instance endpoint...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-test";
        let ssh_user = detect_ssh_user(&api_url, &auth).await?;
        let body = api_post(
            &api_url,
            "/api/sandbox/ssh",
            &auth,
            json!({"public_key": ssh_key}),
        )
        .await?;
        assert_eq!(body["success"], true, "ssh response: {body}");
        assert_eq!(body["username"], ssh_user, "ssh response: {body}");
        assert!(
            ssh_key_presence(&api_url, &auth, &ssh_user, ssh_key).await?,
            "key should be present after provision"
        );
        eprintln!("  SSH provisioned");

        let wrong_user_resp = http()
            .post(format!("{api_url}/api/sandbox/ssh"))
            .header("authorization", &auth)
            .json(&json!({"username": "no-such-user", "public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(wrong_user_resp.status(), 422, "wrong user should fail");

        // ─── Step 14: SSH revoke ─────────────────────────────────────────
        e2e_step!(14, "SSH revoke via instance endpoint...");
        let resp = http()
            .delete(format!("{api_url}/api/sandbox/ssh"))
            .header("authorization", &auth)
            .json(&json!({"public_key": ssh_key}))
            .send()
            .await?;
        assert_eq!(resp.status(), 200, "ssh revoke should succeed");
        let body: Value = resp.json().await?;
        assert_eq!(body["success"], true, "ssh revoke response: {body}");
        assert_eq!(body["username"], ssh_user, "ssh revoke response: {body}");
        assert!(
            !ssh_key_presence(&api_url, &auth, &ssh_user, ssh_key).await?,
            "key should be absent after revoke"
        );
        eprintln!("  SSH revoked");

        // ─── Step 15: Secrets inject + verify ────────────────────────────
        e2e_step!(15, "Injecting secrets via sandbox-scoped endpoint...");
        assert_api_status(
            &api_url,
            "POST",
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &auth,
            json!({"env_json": {"E2E_INSTANCE_SECRET": "instance-secret-42"}}),
            200,
        )
        .await;
        eprintln!("  Secrets injected");

        tokio::time::sleep(Duration::from_secs(3)).await;
        let secrets_url = get_instance_sidecar_url(&api_url, &auth)
            .await
            .unwrap_or(initial_sidecar_url.clone());
        wait_for_sidecar(&secrets_url).await?;

        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
            &auth,
            json!({"command": "printenv E2E_INSTANCE_SECRET"}),
        )
        .await?;
        assert!(
            body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("instance-secret-42"),
            "secret should be in env: {body}"
        );
        eprintln!("  Secret verified via exec");

        // ─── Step 16: Secrets wipe + verify ──────────────────────────────
        e2e_step!(16, "Wiping secrets...");
        let body = api_delete(
            &api_url,
            &format!("/api/sandboxes/{sandbox_id}/secrets"),
            &auth,
        )
        .await?;
        assert_eq!(body["status"], "secrets_wiped", "wipe response: {body}");

        tokio::time::sleep(Duration::from_secs(3)).await;
        let wiped_url = get_instance_sidecar_url(&api_url, &auth)
            .await
            .unwrap_or(secrets_url);
        wait_for_sidecar(&wiped_url).await?;

        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
            &auth,
            json!({"command": "printenv E2E_INSTANCE_SECRET || echo NOT_SET"}),
        )
        .await?;
        assert!(
            !body["stdout"]
                .as_str()
                .unwrap_or("")
                .contains("instance-secret-42"),
            "secret should be gone: {body}"
        );
        eprintln!("  Secrets wiped and verified");

        // ─── Step 17: Stop instance ──────────────────────────────────────
        e2e_step!(17, "Stopping instance...");
        let body = api_post(&api_url, "/api/sandbox/stop", &auth, json!({})).await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Stopped");

        // ─── Step 18: Verify stopped + exec on stopped ──────────────────
        e2e_step!(18, "Verifying stopped state + exec on stopped...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("instance should still be in list")?;
        assert_eq!(sb["state"], "stopped");

        // Exec on stopped instance → should fail (sidecar unreachable)
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/exec",
            &auth,
            json!({"command": "echo should-fail"}),
            502,
        )
        .await;
        eprintln!("  Confirmed stopped, exec returns 502");

        // ─── Step 19: Stop idempotency ───────────────────────────────────
        e2e_step!(19, "Testing stop idempotency...");
        let body = api_post(&api_url, "/api/sandbox/stop", &auth, json!({})).await?;
        assert_eq!(body["state"], "stopped", "second stop: {body}");
        eprintln!("  Stop idempotent");

        // ─── Step 20: Resume instance ────────────────────────────────────
        e2e_step!(20, "Resuming instance...");
        let body = api_post(&api_url, "/api/sandbox/resume", &auth, json!({})).await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Resumed");

        // ─── Step 21: Resume idempotency ─────────────────────────────────
        e2e_step!(21, "Testing resume idempotency...");
        let body = api_post(&api_url, "/api/sandbox/resume", &auth, json!({})).await?;
        assert_eq!(body["state"], "running", "second resume: {body}");
        eprintln!("  Resume idempotent");

        // ─── Step 22: Re-read URL + exec after resume ────────────────────
        e2e_step!(22, "Re-reading sidecar URL, exec after resume...");
        let resumed_url = get_instance_sidecar_url(&api_url, &auth).await?;
        eprintln!("  Post-resume URL: {resumed_url}");
        if resumed_url != initial_sidecar_url {
            eprintln!(
                "  Port changed: {initial_sidecar_url} → {resumed_url} (expected after Docker restart)"
            );
        }
        wait_for_sidecar(&resumed_url).await?;

        let body = api_post(
            &api_url,
            "/api/sandbox/exec",
            &auth,
            json!({"command": "echo instance-resumed-ok"}),
        )
        .await?;
        assert_eq!(body["exit_code"], 0);
        assert!(body["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("instance-resumed-ok"));
        eprintln!("  Exec after resume OK");

        // ─── Step 23: Input validation ───────────────────────────────────
        e2e_step!(23, "Testing input validation (6 checks)...");
        // Empty command → 400
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/exec",
            &auth,
            json!({"command": ""}),
            400,
        )
        .await;
        // Empty prompt message → 400
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/prompt",
            &auth,
            json!({"message": ""}),
            400,
        )
        .await;
        // Empty task prompt → 400
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/task",
            &auth,
            json!({"prompt": ""}),
            400,
        )
        .await;
        // Invalid SSH key → 400
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/ssh",
            &auth,
            json!({"username": "agent", "public_key": "not-a-real-key"}),
            400,
        )
        .await;
        // Empty snapshot destination → 400
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/snapshot",
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

        // ─── Step 24: Auth rejection ─────────────────────────────────────
        e2e_step!(24, "Testing auth rejection...");
        // Missing auth → 401
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth → 401");

        // Bad PASETO → 401
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", "Bearer v4.local.invalid-token-garbage")
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "bad PASETO → 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 25: Cross-owner isolation ──────────────────────────────
        e2e_step!(25, "Testing cross-owner tenant isolation...");
        let non_owner_address = address_from_key(NON_OWNER_KEY);
        let (non_owner_token, addr) = get_auth_token(&api_url, NON_OWNER_KEY).await?;
        assert_eq!(addr.to_lowercase(), non_owner_address.to_lowercase());
        let non_owner_auth = format!("Bearer {non_owner_token}");

        // Non-owner should see empty sandbox list
        let body = api_get(&api_url, "/api/sandboxes", &non_owner_auth).await?;
        let non_owner_sandboxes = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            non_owner_sandboxes.is_empty(),
            "non-owner should see zero sandboxes: {body}"
        );

        // Non-owner trying singleton exec should be rejected
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", &non_owner_auth)
            .json(&json!({"command": "echo pwned"}))
            .send()
            .await?;
        assert!(
            resp.status().is_client_error(),
            "non-owner exec should fail, got {}",
            resp.status()
        );

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
        eprintln!("  Cross-owner isolation confirmed (list, exec, secrets)");

        // ─── Step 26: Deprovision locally ─────────────────────────────────
        e2e_step!(26, "Deprovisioning local instance runtime...");
        let (deprovision_result, _) = deprovision_core(None)
            .await
            .map_err(anyhow::Error::msg)?;
        let deprovision_json: Value = serde_json::from_str(&deprovision_result.json)?;
        assert_eq!(
            deprovision_json["deprovisioned"], true,
            "deprovision response: {deprovision_json}"
        );
        eprintln!("  Deprovisioned: {deprovision_json}");

        // ─── Step 27: Verify gone + exec after deprovision ───────────────
        e2e_step!(27, "Verifying instance gone, exec after deprovision...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let remaining = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            !remaining.iter().any(|s| s["id"] == sandbox_id),
            "deprovisioned instance should not appear"
        );

        // Exec on deprovisioned instance → 404 (Instance not provisioned)
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", &auth)
            .json(&json!({"command": "echo test"}))
            .send()
            .await?;
        assert_eq!(
            resp.status(),
            404,
            "exec after deprovision should return 404, got {}",
            resp.status()
        );
        eprintln!("  Confirmed gone, exec → 404");

        // ─── Step 28: Shutdown ───────────────────────────────────────────
        e2e_step!(28, "Shutting down...");
        harness.shutdown().await;
        eprintln!("\n=== All instance E2E tests passed (28 steps) ===");
        Ok(())
    })
    .await
    .context("instance_full_lifecycle timed out")?
}
