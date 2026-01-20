use ai_agent_sandbox_blueprint_lib::{
    BatchExecRequest, JOB_BATCH_EXEC, JOB_EXEC, JOB_PROMPT, JOB_SANDBOX_CREATE, JOB_TASK,
    JOB_WORKFLOW_CANCEL, JOB_WORKFLOW_CREATE, JOB_WORKFLOW_TRIGGER, JsonResponse,
    SandboxCreateRequest, SandboxExecRequest, SandboxPromptRequest, SandboxTaskRequest,
    WorkflowControlRequest, WorkflowCreateRequest, router,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::sol_types::SolValue;
use once_cell::sync::Lazy;
use std::sync::Once;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);
const JOB_RESULT_TIMEOUT: Duration = Duration::from_secs(180);

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));
static LOG_INIT: Once = Once::new();

fn setup_log() {
    LOG_INIT.call_once(|| {
        let _ = tracing_subscriber::fmt::try_init();
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runs_sandbox_jobs_end_to_end() -> Result<()> {
    setup_log();
    let guard = HARNESS_LOCK.lock().await;
    let result = timeout(ANVIL_TEST_TIMEOUT, async {
        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };

        let mock_server = MockServer::start().await;
        unsafe {
            std::env::set_var("SIDECAR_MOCK_URL", mock_server.uri());
        }

        Mock::given(method("POST"))
            .and(path("/exec"))
            .and(header("authorization", "Bearer sandbox-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exitCode": 0,
                "stdout": "ok",
                "stderr": ""
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .and(header("authorization", "Bearer sandbox-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true,
                "response": "done",
                "traceId": "trace-123",
                "durationMs": 42,
                "usage": {"inputTokens": 10, "outputTokens": 5},
                "sessionId": "session-abc"
            })))
            .mount(&mock_server)
            .await;

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
            sidecar_token: "sandbox-token".to_string(),
        }
        .abi_encode();

        let create_submission = harness
            .submit_job(JOB_SANDBOX_CREATE, Bytes::from(create_payload))
            .await?;
        let create_output = harness
            .wait_for_job_result_with_deadline(create_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let create_receipt = JsonResponse::abi_decode(&create_output)?;
        let create_json: serde_json::Value = serde_json::from_str(&create_receipt.json)
            .context("sandbox create response must be json")?;
        let sidecar_url = create_json
            .get("sidecarUrl")
            .and_then(|value| value.as_str())
            .context("missing sidecarUrl")?
            .to_string();

        let exec_payload = SandboxExecRequest {
            sidecar_url: sidecar_url.clone(),
            command: "echo ok".to_string(),
            cwd: "".to_string(),
            env_json: "".to_string(),
            timeout_ms: 1000,
            sidecar_token: "sandbox-token".to_string(),
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
            sidecar_token: "sandbox-token".to_string(),
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
        assert_eq!(prompt_receipt.response, "done");

        let task_payload = SandboxTaskRequest {
            sidecar_url: sidecar_url.clone(),
            prompt: "do work".to_string(),
            session_id: "session-override".to_string(),
            max_turns: 2,
            model: "".to_string(),
            context_json: "".to_string(),
            timeout_ms: 0,
            sidecar_token: "sandbox-token".to_string(),
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
        assert_eq!(task_receipt.result, "done");
        assert_eq!(task_receipt.session_id, "session-abc");

        let batch_payload = BatchExecRequest {
            sidecar_urls: vec![sidecar_url.clone(), sidecar_url.clone()],
            sidecar_tokens: vec!["sandbox-token".to_string(), "sandbox-token".to_string()],
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
                "{{\"sidecar_url\":\"{}\",\"prompt\":\"run\",\"sidecar_token\":\"sandbox-token\"}}",
                sidecar_url
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
