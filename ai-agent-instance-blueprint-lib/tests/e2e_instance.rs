//! End-to-end test: Instance blueprint lifecycle through Tangle + operator API.
//!
//! This exercises the FULL production path for the instance blueprint:
//!   1. Provision instance via Tangle job (BlueprintHarness → Anvil → Runner → Docker)
//!   2. Start operator API server
//!   3. Authenticate via EIP-191 challenge → PASETO session token
//!   4. Exercise every instance operator API endpoint against the real sidecar
//!   5. Verify cross-owner tenant isolation
//!   6. Deprovision via Tangle job
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
// Test: Full instance lifecycle with on-chain verification
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

        // ─── Step 2: Provision instance via Tangle ───────────────────────
        e2e_step!(2, "Submitting JOB_PROVISION via Tangle...");
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
            .await
            .context("provision job result not received")?;

        // Verify on-chain result is valid ABI
        let provision_receipt = ProvisionOutput::abi_decode(&provision_output)
            .context("failed to ABI-decode ProvisionOutput from on-chain result")?;
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
        eprintln!("  Found instance, state=running");

        // ─── Step 7: Exec via singleton endpoint ─────────────────────────
        e2e_step!(7, "Exec via instance endpoint...");
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
        eprintln!("  Exec OK");

        // ─── Step 8: SSH provision ───────────────────────────────────────
        e2e_step!(8, "SSH provision via instance endpoint...");
        let ssh_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBp9pDAVl8TpDBLVnpXjAIRxMf3K+m6UPlv3VBMbRp2o e2e-test";
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/ssh",
            &auth,
            json!({"username": "agent", "public_key": ssh_key}),
            200,
        )
        .await;
        eprintln!("  SSH provisioned");

        // ─── Step 9: SSH revoke ──────────────────────────────────────────
        e2e_step!(9, "SSH revoke via instance endpoint...");
        assert_api_status(
            &api_url,
            "DELETE",
            "/api/sandbox/ssh",
            &auth,
            json!({"username": "agent", "public_key": ssh_key}),
            200,
        )
        .await;
        eprintln!("  SSH revoked");

        // ─── Step 10: Stop instance ──────────────────────────────────────
        e2e_step!(10, "Stopping instance...");
        let body = api_post(&api_url, "/api/sandbox/stop", &auth, json!({})).await?;
        assert_eq!(body["state"], "stopped");
        eprintln!("  Stopped");

        // ─── Step 11: Verify stopped in list ─────────────────────────────
        e2e_step!(11, "Verifying stopped state...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let sb = body["sandboxes"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == sandbox_id))
            .context("instance should still be in list")?;
        assert_eq!(sb["state"], "stopped");
        eprintln!("  Confirmed stopped");

        // ─── Step 12: Resume instance ────────────────────────────────────
        e2e_step!(12, "Resuming instance...");
        let body = api_post(&api_url, "/api/sandbox/resume", &auth, json!({})).await?;
        assert_eq!(body["state"], "running");
        eprintln!("  Resumed");

        // ─── Step 13: Re-read sidecar URL (Docker assigns new ports) ─────
        e2e_step!(13, "Re-reading sidecar URL after resume...");
        let resumed_url = get_instance_sidecar_url(&api_url, &auth).await?;
        eprintln!("  Post-resume URL: {resumed_url}");
        if resumed_url != initial_sidecar_url {
            eprintln!(
                "  Port changed: {} → {} (expected after Docker restart)",
                initial_sidecar_url, resumed_url
            );
        }
        wait_for_sidecar(&resumed_url).await?;

        // ─── Step 14: Exec after resume ──────────────────────────────────
        e2e_step!(14, "Exec after resume...");
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

        // ─── Step 15: Input validation ───────────────────────────────────
        e2e_step!(15, "Testing input validation...");
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/exec",
            &auth,
            json!({"command": ""}),
            400,
        )
        .await;
        assert_api_status(
            &api_url,
            "POST",
            "/api/sandbox/ssh",
            &auth,
            json!({"username": "agent", "public_key": "not-a-real-key"}),
            400,
        )
        .await;
        eprintln!("  Input validation OK");

        // ─── Step 16: Auth rejection ─────────────────────────────────────
        e2e_step!(16, "Testing auth rejection...");
        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "missing auth → 401");

        let resp = http()
            .post(format!("{api_url}/api/sandbox/exec"))
            .header("authorization", "Bearer v4.local.invalid-token-garbage")
            .json(&json!({"command": "echo fail"}))
            .send()
            .await?;
        assert_eq!(resp.status(), 401, "bad PASETO → 401");
        eprintln!("  Auth rejection OK");

        // ─── Step 17: Cross-owner isolation ──────────────────────────────
        e2e_step!(17, "Testing cross-owner tenant isolation...");
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
        eprintln!("  Cross-owner isolation confirmed");

        // ─── Step 18: Deprovision via Tangle ─────────────────────────────
        e2e_step!(18, "Deprovisioning via Tangle...");
        let deprovision_payload = JsonResponse {
            json: json!({"action": "deprovision"}).to_string(),
        }
        .abi_encode();
        let deprovision_sub = harness
            .submit_job(JOB_DEPROVISION, Bytes::from(deprovision_payload))
            .await?;
        let deprovision_output = harness
            .wait_for_job_result_with_deadline(deprovision_sub, JOB_RESULT_TIMEOUT)
            .await
            .context("deprovision job result not received")?;

        let deprovision_result = JsonResponse::abi_decode(&deprovision_output)
            .context("failed to decode deprovision result")?;
        let deprovision_json: Value = serde_json::from_str(&deprovision_result.json)?;
        assert_eq!(
            deprovision_json["deprovisioned"], true,
            "deprovision response: {deprovision_json}"
        );
        eprintln!("  Deprovisioned: {deprovision_json}");

        // ─── Step 19: Verify gone ────────────────────────────────────────
        e2e_step!(19, "Verifying instance gone from list...");
        let body = api_get(&api_url, "/api/sandboxes", &auth).await?;
        let remaining = body["sandboxes"]
            .as_array()
            .context("sandboxes array")?;
        assert!(
            !remaining.iter().any(|s| s["id"] == sandbox_id),
            "deprovisioned instance should not appear"
        );
        eprintln!("  Confirmed gone");

        // ─── Step 20: Shutdown ───────────────────────────────────────────
        e2e_step!(20, "Shutting down...");
        harness.shutdown().await;
        eprintln!("\n=== All instance E2E tests passed (20 steps) ===");
        Ok(())
    })
    .await
    .context("instance_full_lifecycle timed out")?
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP assertion helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn api_get(api_url: &str, path: &str, auth: &str) -> Result<Value> {
    let resp = http()
        .get(format!("{api_url}{path}"))
        .header("authorization", auth)
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    anyhow::ensure!(status.is_success(), "GET {path} returned {status}: {body}");
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
