use ai_agent_sandbox_blueprint_lib::{
    BatchExecRequest, JOB_BATCH_EXEC, JOB_EXEC, JOB_PROMPT, JOB_SANDBOX_CREATE, JOB_TASK,
    JOB_WORKFLOW_CANCEL, JOB_WORKFLOW_CREATE, JOB_WORKFLOW_TRIGGER, JsonResponse,
    SandboxCreateOutput, SandboxCreateRequest, SandboxExecRequest, SandboxPromptRequest,
    SandboxTaskRequest, WorkflowControlRequest, WorkflowCreateRequest, router,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::sol_types::SolValue;
use once_cell::sync::Lazy;
use std::net::TcpListener;
use std::sync::Once;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);
const JOB_RESULT_TIMEOUT: Duration = Duration::from_secs(180);

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));
static LOG_INIT: Once = Once::new();

fn setup_log() {
    LOG_INIT.call_once(|| {
        let _ = tracing_subscriber::fmt::try_init();
    });
}

/// Set up environment for the sidecar runtime config.
/// Must be called before the first SidecarRuntimeConfig::load().
fn setup_sidecar_env() {
    let image =
        std::env::var("SIDECAR_IMAGE").unwrap_or_else(|_| "tangle-sidecar:local".to_string());
    unsafe {
        std::env::set_var("SIDECAR_IMAGE", &image);
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "60");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runs_sandbox_jobs_end_to_end() -> Result<()> {
    setup_log();
    let guard = HARNESS_LOCK.lock().await;
    let result = timeout(ANVIL_TEST_TIMEOUT, async {
        if std::env::var("SIDECAR_E2E").ok().as_deref() != Some("1") {
            return Ok(());
        }

        setup_sidecar_env();

        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };

        let create_payload = SandboxCreateRequest {
            name: "agent-sandbox".to_string(),
            image: "agent-dev".to_string(),
            stack: "default".to_string(),
            agent_identifier: "default-agent".to_string(),
            env_json: "{}".to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: true,
            ssh_public_key: "ssh-ed25519 AAA test".to_string(),
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

        let create_submission = harness
            .submit_job(JOB_SANDBOX_CREATE, Bytes::from(create_payload))
            .await?;
        let create_output = harness
            .wait_for_job_result_with_deadline(create_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let create_receipt = SandboxCreateOutput::abi_decode(&create_output)?;
        let create_json: serde_json::Value = serde_json::from_str(&create_receipt.json)
            .context("sandbox create response must be json")?;
        let sidecar_url = create_json
            .get("sidecarUrl")
            .and_then(|value| value.as_str())
            .context("missing sidecarUrl")?
            .to_string();
        eprintln!("Sandbox created: id={}, url={sidecar_url}", create_receipt.sandboxId);

        // ---------------------------------------------------------------
        // Operator API verification: start an API server and check state
        // ---------------------------------------------------------------
        let api_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_port = api_listener.local_addr().unwrap().port();
        drop(api_listener);

        let api_app = sandbox_runtime::operator_api::operator_api_router();
        let api_listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
            .await
            .unwrap();
        let _api_handle = tokio::spawn(async move {
            axum::serve(api_listener, api_app).await.ok();
        });
        let api_url = format!("http://127.0.0.1:{api_port}");

        // Wait for operator API to be ready.
        {
            let api_client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let api_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                if tokio::time::Instant::now() > api_deadline {
                    anyhow::bail!("Operator API not ready within 5s");
                }
                if let Ok(r) = api_client.get(format!("{api_url}/api/provisions")).send().await {
                    if r.status().is_success() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        // Check provision progress: sandbox_create wires provision tracking via CallId.
        // The provision should exist and be in Ready state (or exist at all).
        let provisions_resp = reqwest::Client::new()
            .get(format!("{api_url}/api/provisions"))
            .send()
            .await?;
        let provisions_body: serde_json::Value = provisions_resp.json().await?;
        let provisions = provisions_body["provisions"].as_array().context("provisions array")?;
        eprintln!("Provisions after sandbox_create: {} entries", provisions.len());
        // At least one provision should exist from the sandbox_create job.
        if !provisions.is_empty() {
            let last = provisions.last().unwrap();
            eprintln!(
                "Latest provision: call_id={}, phase={}, sandbox_id={}",
                last["call_id"], last["phase"], last["sandbox_id"]
            );
            // The sandbox_create handler should have marked it Ready.
            assert_eq!(
                last["phase"], "ready",
                "provision should be Ready after sandbox_create"
            );
        }

        // Session auth flow via operator API.
        {
            use k256::ecdsa::SigningKey;
            use rand::rngs::OsRng;

            let signing_key = SigningKey::random(&mut OsRng);
            let verifying_key = signing_key.verifying_key();
            let pubkey_bytes = verifying_key.to_encoded_point(false);
            let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
            let address_hash = keccak256(pubkey_uncompressed);
            let expected_address = format!("0x{}", hex::encode(&address_hash[12..]));

            let challenge_resp = reqwest::Client::new()
                .post(format!("{api_url}/api/auth/challenge"))
                .send()
                .await?;
            assert_eq!(challenge_resp.status(), 200);
            let challenge: serde_json::Value = challenge_resp.json().await?;
            let nonce = challenge["nonce"].as_str().context("nonce")?;
            let message = challenge["message"].as_str().context("message")?;

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

            let session_resp = reqwest::Client::new()
                .post(format!("{api_url}/api/auth/session"))
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "nonce": nonce,
                    "signature": sig_hex,
                }))
                .send()
                .await?;
            assert_eq!(session_resp.status(), 200, "session exchange should succeed");
            let session: serde_json::Value = session_resp.json().await?;
            let token = session["token"].as_str().context("token")?;
            let address = session["address"].as_str().context("address")?;
            assert!(token.starts_with("v4.local."), "PASETO v4 token");
            assert_eq!(address, expected_address);
            eprintln!("Session auth OK: address={address}");
        }

        // Wait for the sidecar to become healthy before sending jobs
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let health_deadline =
            tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            if tokio::time::Instant::now() > health_deadline {
                anyhow::bail!(
                    "Sidecar not healthy within 60s at {sidecar_url}"
                );
            }
            if let Ok(resp) = client.get(format!("{sidecar_url}/health")).send().await {
                if resp.status().is_success() {
                    eprintln!("Sidecar healthy at {sidecar_url}");
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let exec_payload = SandboxExecRequest {
            sidecar_url: sidecar_url.clone(),
            command: "echo ok".to_string(),
            cwd: "".to_string(),
            env_json: "".to_string(),
            timeout_ms: 30000,
        }
        .abi_encode();
        let exec_submission = harness
            .submit_job(JOB_EXEC, Bytes::from(exec_payload))
            .await?;
        let exec_output = harness
            .wait_for_job_result_with_deadline(exec_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let exec_receipt =
            ai_agent_sandbox_blueprint_lib::SandboxExecResponse::abi_decode(&exec_output)?;
        assert_eq!(exec_receipt.exit_code, 0);
        assert_eq!(exec_receipt.stdout, "ok");

        let prompt_payload = SandboxPromptRequest {
            sidecar_url: sidecar_url.clone(),
            message: "hello".to_string(),
            session_id: "".to_string(),
            model: "".to_string(),
            context_json: "".to_string(),
            timeout_ms: 0,
        }
        .abi_encode();
        let prompt_submission = harness
            .submit_job(JOB_PROMPT, Bytes::from(prompt_payload))
            .await?;
        let prompt_output = harness
            .wait_for_job_result_with_deadline(prompt_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let prompt_receipt =
            ai_agent_sandbox_blueprint_lib::SandboxPromptResponse::abi_decode(&prompt_output)?;
        assert!(prompt_receipt.success);
        assert!(!prompt_receipt.response.is_empty());

        let task_payload = SandboxTaskRequest {
            sidecar_url: sidecar_url.clone(),
            prompt: "do work".to_string(),
            session_id: "session-override".to_string(),
            max_turns: 2,
            model: "".to_string(),
            context_json: "".to_string(),
            timeout_ms: 0,
        }
        .abi_encode();
        let task_submission = harness
            .submit_job(JOB_TASK, Bytes::from(task_payload))
            .await?;
        let task_output = harness
            .wait_for_job_result_with_deadline(task_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let task_receipt =
            ai_agent_sandbox_blueprint_lib::SandboxTaskResponse::abi_decode(&task_output)?;
        assert!(task_receipt.success);
        assert!(!task_receipt.result.is_empty());
        assert!(!task_receipt.session_id.is_empty());

        let batch_payload = BatchExecRequest {
            sidecar_urls: vec![sidecar_url.clone(), sidecar_url.clone()],
            command: "ls".to_string(),
            cwd: "".to_string(),
            env_json: "".to_string(),
            timeout_ms: 0,
            parallel: false,
        }
        .abi_encode();
        let batch_submission = harness
            .submit_job(JOB_BATCH_EXEC, Bytes::from(batch_payload))
            .await?;
        let batch_output = harness
            .wait_for_job_result_with_deadline(batch_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let batch_receipt = JsonResponse::abi_decode(&batch_output)?;
        let batch_json: serde_json::Value = serde_json::from_str(&batch_receipt.json)
            .context("batch exec response must be json")?;
        let results = batch_json
            .get("execResults")
            .and_then(|value| value.as_array())
            .context("missing execResults")?;
        assert_eq!(results.len(), 2);

        let workflow_payload = WorkflowCreateRequest {
            name: "daily".to_string(),
            workflow_json: format!(
                "{{\"sidecar_url\":\"{sidecar_url}\",\"prompt\":\"run\",\"sidecar_token\":\"sandbox-token\"}}"
            ),
            trigger_type: "cron".to_string(),
            trigger_config: "0 * * * * *".to_string(),
            sandbox_config_json: "{}".to_string(),
        }
        .abi_encode();
        let workflow_submission = harness
            .submit_job(JOB_WORKFLOW_CREATE, Bytes::from(workflow_payload))
            .await?;
        let workflow_output = harness
            .wait_for_job_result_with_deadline(workflow_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let workflow_receipt = JsonResponse::abi_decode(&workflow_output)?;
        let workflow_json: serde_json::Value = serde_json::from_str(&workflow_receipt.json)
            .context("workflow create response must be json")?;
        let workflow_id = workflow_json
            .get("workflowId")
            .and_then(|value| value.as_u64())
            .context("missing workflowId")?;

        let trigger_payload = WorkflowControlRequest { workflow_id }.abi_encode();
        let trigger_submission = harness
            .submit_job(JOB_WORKFLOW_TRIGGER, Bytes::from(trigger_payload))
            .await?;
        let trigger_output = harness
            .wait_for_job_result_with_deadline(trigger_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let trigger_receipt = JsonResponse::abi_decode(&trigger_output)?;
        let trigger_json: serde_json::Value = serde_json::from_str(&trigger_receipt.json)
            .context("workflow trigger response must be json")?;
        assert!(
            trigger_json
                .get("task")
                .and_then(|task| task.get("success"))
                .and_then(|val| val.as_bool())
                .unwrap_or(false)
        );

        let cancel_payload = WorkflowControlRequest { workflow_id }.abi_encode();
        let cancel_submission = harness
            .submit_job(JOB_WORKFLOW_CANCEL, Bytes::from(cancel_payload))
            .await?;
        let cancel_output = harness
            .wait_for_job_result_with_deadline(cancel_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let cancel_receipt = JsonResponse::abi_decode(&cancel_output)?;
        assert!(cancel_receipt.json.contains("canceled"));

        harness.shutdown().await;
        Ok(())
    })
    .await;

    drop(guard);
    result.context("runs_sandbox_jobs_end_to_end timed out")?
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
                eprintln!("Skipping runs_sandbox_jobs_end_to_end: {err}");
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}
