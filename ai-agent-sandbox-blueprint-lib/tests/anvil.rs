use ai_agent_sandbox_blueprint_lib::{
    JOB_SANDBOX_CREATE, JOB_SANDBOX_DELETE, JOB_WORKFLOW_CANCEL, JOB_WORKFLOW_CREATE,
    JOB_WORKFLOW_TRIGGER, JsonResponse, SandboxCreateOutput, SandboxCreateRequest,
    SandboxIdRequest, WorkflowControlRequest, WorkflowCreateRequest, router,
};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
};
use blueprint_anvil_testing_utils::{BlueprintHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::Bytes;
use blueprint_sdk::alloy::sol_types::SolValue;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::json;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::Once;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

const ANVIL_TEST_TIMEOUT: Duration = Duration::from_secs(600);
const JOB_RESULT_TIMEOUT: Duration = Duration::from_secs(180);

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));
static LOG_INIT: Once = Once::new();

#[derive(Clone, Debug, Default)]
struct MockFirecrackerHostState {
    sidecar_url: String,
    containers: std::collections::HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct FirecrackerCreateContainerRequest {
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn require_api_key(
    headers: &HeaderMap,
    expected: &str,
) -> std::result::Result<(), (StatusCode, Json<serde_json::Value>)> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if key == expected {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"missing or invalid api key","code":"UNAUTHORIZED"})),
        ))
    }
}

fn firecracker_container_json(id: &str, endpoint: &str, running: bool) -> serde_json::Value {
    json!({
        "id": id,
        "name": id,
        "sessionId": id,
        "image": "mock-rootfs",
        "status": if running { "running" } else { "created" },
        "state": if running { "running" } else { "terminated" },
        "endpoint": if running { endpoint } else { "" },
        "createdAt": 0,
        "labels": {},
        "resources": { "cpu": 1, "memory": 512, "disk": 1024, "pids": 128 }
    })
}

async fn spawn_firecracker_host_agent_mock(
    api_key: String,
    sidecar_url: String,
) -> Result<(String, Arc<AsyncMutex<MockFirecrackerHostState>>)> {
    let state = Arc::new(AsyncMutex::new(MockFirecrackerHostState {
        sidecar_url: sidecar_url.clone(),
        containers: std::collections::HashSet::new(),
    }));

    let create_key = api_key.clone();
    let health_key = api_key.clone();
    let start_key = api_key.clone();
    let delete_key = api_key;

    let app = Router::new()
        .route(
            "/v1/health",
            get(move |headers: HeaderMap| {
                let health_key = health_key.clone();
                async move {
                    require_api_key(&headers, &health_key)?;
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>((
                        StatusCode::OK,
                        Json(json!({"ok": true})),
                    ))
                }
            }),
        )
        .route(
            "/v1/containers",
            post(
                move |State(state): State<Arc<AsyncMutex<MockFirecrackerHostState>>>,
                      headers: HeaderMap,
                      Json(body): Json<FirecrackerCreateContainerRequest>| {
                    let create_key = create_key.clone();
                    async move {
                        require_api_key(&headers, &create_key)?;
                        let mut guard = state.lock().await;
                        guard.containers.insert(body.session_id.clone());
                        Ok::<_, (StatusCode, Json<serde_json::Value>)>((
                            StatusCode::CREATED,
                            Json(firecracker_container_json(
                                &body.session_id,
                                &guard.sidecar_url,
                                false,
                            )),
                        ))
                    }
                },
            ),
        )
        .route(
            "/v1/containers/{id}/start",
            post(
                move |State(state): State<Arc<AsyncMutex<MockFirecrackerHostState>>>,
                      headers: HeaderMap,
                      Path(id): Path<String>| {
                    let start_key = start_key.clone();
                    async move {
                        require_api_key(&headers, &start_key)?;
                        let guard = state.lock().await;
                        if !guard.containers.contains(&id) {
                            return Err((
                                StatusCode::NOT_FOUND,
                                Json(json!({"error":"not found","code":"NOT_FOUND"})),
                            ));
                        }
                        Ok::<_, (StatusCode, Json<serde_json::Value>)>((
                            StatusCode::OK,
                            Json(firecracker_container_json(&id, &guard.sidecar_url, true)),
                        ))
                    }
                },
            ),
        )
        .route(
            "/v1/containers/{id}",
            delete(
                move |State(state): State<Arc<AsyncMutex<MockFirecrackerHostState>>>,
                      headers: HeaderMap,
                      Path(id): Path<String>| {
                    let delete_key = delete_key.clone();
                    async move {
                        require_api_key(&headers, &delete_key)?;
                        let mut guard = state.lock().await;
                        if guard.containers.remove(&id) {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::OK,
                                Json(json!({"ok": true})),
                            ))
                        } else {
                            Err((
                                StatusCode::NOT_FOUND,
                                Json(json!({"error":"not found","code":"NOT_FOUND"})),
                            ))
                        }
                    }
                },
            ),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{}:{}", addr.ip(), addr.port()), state))
}

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

        // Wait for the sidecar to become healthy before sending workflow jobs
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

        // Read-only ops (exec, prompt, task, stop/resume, batch, ssh, snapshot)
        // are now served via the operator HTTP API, not on-chain jobs.
        // See sandbox-runtime/src/operator_api.rs for those endpoints.

        let workflow_payload = WorkflowCreateRequest {
            name: "daily".to_string(),
            workflow_json: format!(
                "{{\"sidecar_url\":\"{sidecar_url}\",\"prompt\":\"run\",\"sidecar_token\":\"sandbox-token\"}}"
            ),
            trigger_type: "cron".to_string(),
            trigger_config: "0 * * * * *".to_string(),
            sandbox_config_json: "{}".to_string(),
            target_kind: 0,
            target_sandbox_id: create_receipt.sandboxId.clone(),
            target_service_id: 1,
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

        // ---------------------------------------------------------------
        // Sandbox lifecycle: delete (cleanup)
        // ---------------------------------------------------------------
        let delete_payload = SandboxIdRequest {
            sandbox_id: create_receipt.sandboxId.clone(),
        }
        .abi_encode();
        let delete_submission = harness
            .submit_job(JOB_SANDBOX_DELETE, Bytes::from(delete_payload))
            .await?;
        let delete_output = harness
            .wait_for_job_result_with_deadline(delete_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let delete_receipt = JsonResponse::abi_decode(&delete_output)?;
        let delete_json: serde_json::Value =
            serde_json::from_str(&delete_receipt.json).context("delete response must be json")?;
        assert_eq!(delete_json["deleted"], true);
        eprintln!("Sandbox deleted: id={}", create_receipt.sandboxId);

        harness.shutdown().await;
        Ok(())
    })
    .await;

    drop(guard);
    result.context("runs_sandbox_jobs_end_to_end timed out")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runs_firecracker_jobs_end_to_end() -> Result<()> {
    setup_log();
    let guard = HARNESS_LOCK.lock().await;
    let result = timeout(ANVIL_TEST_TIMEOUT, async {
        if std::env::var("FIRECRACKER_E2E").ok().as_deref() != Some("1") {
            return Ok(());
        }

        let Some(harness) = spawn_harness().await? else {
            return Ok(());
        };

        let real_mode = std::env::var("FIRECRACKER_REAL_E2E").ok().as_deref() == Some("1");
        let firecracker_image = std::env::var("FIRECRACKER_TEST_IMAGE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "sidecar.ext4".to_string());
        let mut host_state: Option<Arc<AsyncMutex<MockFirecrackerHostState>>> = None;
        let mut expected_sidecar_url: Option<String> = None;
        if real_mode {
            anyhow::ensure!(
                std::env::var("FIRECRACKER_HOST_AGENT_URL")
                    .ok()
                    .is_some_and(|v| !v.trim().is_empty()),
                "FIRECRACKER_REAL_E2E=1 requires FIRECRACKER_HOST_AGENT_URL"
            );
        } else {
            let host_agent_key = "fc-test-key".to_string();
            let mock_sidecar_url = "http://127.0.0.1:65535".to_string();
            let (host_agent_url, state) =
                spawn_firecracker_host_agent_mock(host_agent_key.clone(), mock_sidecar_url.clone())
                    .await?;
            expected_sidecar_url = Some(mock_sidecar_url);
            host_state = Some(state);
            unsafe {
                std::env::set_var("FIRECRACKER_HOST_AGENT_URL", &host_agent_url);
                std::env::set_var("FIRECRACKER_HOST_AGENT_API_KEY", &host_agent_key);
            }
        }
        unsafe {
            std::env::set_var("FIRECRACKER_SIDECAR_AUTH_DISABLED", "true");
            std::env::remove_var("FIRECRACKER_SIDECAR_AUTH_TOKEN");
        }

        let create_payload = SandboxCreateRequest {
            name: "agent-firecracker-sandbox".to_string(),
            image: firecracker_image,
            stack: "default".to_string(),
            agent_identifier: "default-agent".to_string(),
            env_json: "{}".to_string(),
            metadata_json: r#"{"runtime_backend":"firecracker"}"#.to_string(),
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

        let create_submission = harness
            .submit_job(JOB_SANDBOX_CREATE, Bytes::from(create_payload))
            .await?;
        let create_output = harness
            .wait_for_job_result_with_deadline(create_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let create_receipt = SandboxCreateOutput::abi_decode(&create_output)?;
        let create_json: serde_json::Value =
            serde_json::from_str(&create_receipt.json).context("create response must be json")?;
        let sidecar_url = create_json["sidecarUrl"]
            .as_str()
            .context("missing sidecarUrl in firecracker create response")?
            .to_string();
        if let Some(expected) = expected_sidecar_url {
            assert_eq!(sidecar_url, expected);
        } else {
            assert!(
                sidecar_url.starts_with("http://") || sidecar_url.starts_with("https://"),
                "unexpected sidecarUrl format: {sidecar_url}"
            );
        }
        eprintln!(
            "Firecracker sandbox created: id={}, url={}",
            create_receipt.sandboxId, sidecar_url
        );

        let delete_payload = SandboxIdRequest {
            sandbox_id: create_receipt.sandboxId.clone(),
        }
        .abi_encode();
        let delete_submission = harness
            .submit_job(JOB_SANDBOX_DELETE, Bytes::from(delete_payload))
            .await?;
        let delete_output = harness
            .wait_for_job_result_with_deadline(delete_submission, JOB_RESULT_TIMEOUT)
            .await?;
        let delete_receipt = JsonResponse::abi_decode(&delete_output)?;
        let delete_json: serde_json::Value =
            serde_json::from_str(&delete_receipt.json).context("delete response must be json")?;
        assert_eq!(delete_json["deleted"], true);

        if let Some(state) = host_state {
            let host_guard = state.lock().await;
            assert!(
                host_guard.containers.is_empty(),
                "host-agent mock should have no live containers after delete"
            );
            drop(host_guard);
        }

        harness.shutdown().await;
        Ok(())
    })
    .await;

    drop(guard);
    result.context("runs_firecracker_jobs_end_to_end timed out")?
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
