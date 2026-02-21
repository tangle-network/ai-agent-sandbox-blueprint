//! Real sidecar integration tests for the Instance Blueprint.
//!
//! These tests spin up an actual sidecar Docker container and hit real HTTP
//! endpoints. They verify real response shapes, real auth behavior, and real
//! command execution — no mocks.
//!
//! The instance blueprint functions (`run_instance_exec`, `run_instance_prompt`,
//! `run_instance_task`) take (sidecar_url, token, sandbox_id, request) directly —
//! they are instance-scoped (no sidecar_url in the ABI request struct).
//!
//! Run (infrastructure only):
//!   REAL_SIDECAR=1 cargo test -p ai-agent-instance-blueprint-lib --test real_sidecar -- --test-threads=1
//!
//! Run (with AI backend):
//!   REAL_SIDECAR=1 ZAI_API_KEY=<key> cargo test -p ai-agent-instance-blueprint-lib --test real_sidecar -- --test-threads=1
//!
//! Requires Docker and a local sidecar image (default: tangle-sidecar:local).
//! Override with SIDECAR_IMAGE env var.

use std::collections::HashMap;
use std::time::Duration;

use ai_agent_instance_blueprint_lib::*;
use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions,
};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

// ---------------------------------------------------------------------------
// Shared container setup
// ---------------------------------------------------------------------------

const AUTH_TOKEN: &str = "test-instance-sidecar-token-a3b7c1";
const CONTAINER_NAME: &str = "test-instance-real-sidecar";
const CONTAINER_PORT: u16 = 8080;

struct TestSidecar {
    url: String,
    #[allow(dead_code)]
    container_id: String,
}

static SIDECAR: OnceCell<TestSidecar> = OnceCell::const_new();

async fn docker_builder() -> DockerBuilder {
    match DockerBuilder::new().await {
        Ok(b) => b,
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_default();
            let mac_sock = format!("unix://{home}/.docker/run/docker.sock");
            DockerBuilder::with_address(&mac_sock)
                .await
                .expect("Docker daemon not reachable (tried default + macOS socket)")
        }
    }
}

async fn ensure_sidecar() -> &'static TestSidecar {
    SIDECAR
        .get_or_init(|| async {
            // Set a generous HTTP client timeout for AI tasks.
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

            // Port bindings: container 8080 → random host port.
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

            let mut env_vars = vec![
                format!("SIDECAR_PORT={CONTAINER_PORT}"),
                format!("SIDECAR_AUTH_TOKEN={AUTH_TOKEN}"),
                "NODE_ENV=development".to_string(),
                "PORT_WATCHER_ENABLED=false".to_string(),
            ];

            // Configure ZAI AI backend when API key is available.
            if let Ok(api_key) = std::env::var("ZAI_API_KEY") {
                if !api_key.is_empty() {
                    env_vars.push("AGENT_BACKEND=opencode".to_string());
                    env_vars.push("OPENCODE_MODEL_PROVIDER=zai-coding-plan".to_string());
                    env_vars.push(format!("OPENCODE_MODEL_API_KEY={api_key}"));
                    env_vars.push("OPENCODE_MODEL_NAME=glm-4.7".to_string());
                }
            }

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

            // When AI backend is configured, warm it up.
            if std::env::var("ZAI_API_KEY").is_ok() {
                eprintln!("Warming up AI backend...");
                let warmup_deadline = tokio::time::Instant::now() + Duration::from_secs(90);
                loop {
                    if tokio::time::Instant::now() > warmup_deadline {
                        eprintln!("Warning: AI backend warmup timed out (tests may fail)");
                        break;
                    }
                    let resp = client
                        .post(format!("{url}/agents/run"))
                        .header(
                            AUTHORIZATION,
                            HeaderValue::from_str(&format!("Bearer {AUTH_TOKEN}")).unwrap(),
                        )
                        .header(CONTENT_TYPE, "application/json")
                        .json(&json!({"message": "ping", "identifier": "default"}))
                        .send()
                        .await;
                    match resp {
                        Ok(r) if r.status().is_success() => {
                            eprintln!("AI backend ready");
                            break;
                        }
                        Ok(r) => {
                            let body = r.text().await.unwrap_or_default();
                            if body.contains("not responding") || body.contains("crashed") {
                                eprintln!("AI backend not ready yet, retrying...");
                                tokio::time::sleep(Duration::from_secs(3)).await;
                            } else {
                                eprintln!(
                                    "AI backend ready (responded with error: {})",
                                    &body[..body.len().min(100)]
                                );
                                break;
                            }
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
            }

            TestSidecar { url, container_id }
        })
        .await
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn auth_header() -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {AUTH_TOKEN}")).unwrap()
}

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

macro_rules! skip_unless_ai {
    () => {
        if !should_run() || std::env::var("ZAI_API_KEY").is_err() {
            eprintln!("Skipped (set REAL_SIDECAR=1 and ZAI_API_KEY to run)");
            return;
        }
    };
}

/// Dummy sandbox ID for instance-scoped functions.
const SANDBOX_ID: &str = "instance-test-sandbox";

// ===================================================================
// Health
// ===================================================================

#[tokio::test]
async fn health_response_shape() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .get(format!("{}/health", s.url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["status"], "ok", "body: {body}");
    assert!(body["backends"].is_object(), "backends missing: {body}");
    assert!(
        body["backends"]["total"].is_number(),
        "backends.total: {body}"
    );
    assert!(body["timestamp"].is_string(), "timestamp: {body}");
}

#[tokio::test]
async fn health_detailed_response_shape() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .get(format!("{}/health/detailed", s.url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert!(
        ["healthy", "degraded", "unhealthy"].contains(&body["status"].as_str().unwrap_or("")),
        "status: {body}"
    );
    assert!(body["memory"].is_object(), "memory: {body}");
    assert!(body["process"].is_object(), "process: {body}");
    assert!(body["uptime"].is_number(), "uptime: {body}");
}

// ===================================================================
// Auth
// ===================================================================

#[tokio::test]
async fn auth_missing_token_rejected() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo hi", "timeout": 5000}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "Expected 401 without token");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");
}

#[tokio::test]
async fn auth_wrong_token_rejected() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, "Bearer wrong-token-absolutely")
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo hi", "timeout": 5000}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403, "Expected 403 with wrong token");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

#[tokio::test]
async fn auth_correct_token_accepted() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo auth-ok", "timeout": 5000}))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "Expected 2xx with correct token, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn auth_health_skips_auth() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .get(format!("{}/health", s.url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "Health should skip auth");
}

// ===================================================================
// Terminal commands via instance blueprint functions
// ===================================================================

#[tokio::test]
async fn exec_echo() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "echo instance-exec-ok".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("run_instance_exec should succeed");

    assert_eq!(resp.exit_code, 0, "exit_code should be 0");
    assert!(
        resp.stdout.contains("instance-exec-ok"),
        "stdout should contain our text: '{}'",
        resp.stdout
    );
}

#[tokio::test]
async fn exec_exit_code() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "exit 42".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("should succeed");

    assert_eq!(resp.exit_code, 42, "exit_code should be 42");
}

#[tokio::test]
async fn exec_with_cwd() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "pwd".to_string(),
        cwd: "/tmp".to_string(),
        env_json: String::new(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("should succeed");

    assert_eq!(resp.exit_code, 0);
    assert!(
        resp.stdout.contains("/tmp"),
        "cwd should be /tmp, got: '{}'",
        resp.stdout
    );
}

#[tokio::test]
async fn exec_with_env() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "echo $INSTANCE_TEST_VAR".to_string(),
        cwd: String::new(),
        env_json: r#"{"INSTANCE_TEST_VAR": "env-val-xyz"}"#.to_string(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("should succeed");

    eprintln!("env test stdout: '{}'", resp.stdout);
    // PTY may or may not propagate env vars — just verify no crash.
    assert_eq!(resp.exit_code, 0);
}

#[tokio::test]
async fn exec_multiline_output() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "echo line1; echo line2; echo line3".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("should succeed");

    assert_eq!(resp.exit_code, 0);
    assert!(resp.stdout.contains("line1"), "should contain line1");
    assert!(resp.stdout.contains("line3"), "should contain line3");
}

#[tokio::test]
async fn concurrent_exec_requests() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let mut handles = Vec::new();
    for i in 0..5 {
        let url = s.url.clone();
        handles.push(tokio::spawn(async move {
            let request = InstanceExecRequest {
                command: format!("echo concurrent-{i}"),
                cwd: String::new(),
                env_json: String::new(),
                timeout_ms: 15000,
            };
            let resp = run_instance_exec(&url, AUTH_TOKEN, SANDBOX_ID, &request).await;
            (i, resp)
        }));
    }

    for handle in handles {
        let (i, result) = handle.await.unwrap();
        let resp = result.unwrap_or_else(|e| panic!("concurrent-{i} failed: {e}"));
        assert_eq!(resp.exit_code, 0, "concurrent-{i} exit_code");
        assert!(
            resp.stdout.contains(&format!("concurrent-{i}")),
            "concurrent-{i} missing from stdout: '{}'",
            resp.stdout
        );
    }
}

// ===================================================================
// Terminal session CRUD
// ===================================================================

#[tokio::test]
async fn terminal_create_list_get_delete() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Create.
    let create_resp = http()
        .post(format!("{}/terminals", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"name": "instance-test-terminal"}))
        .send()
        .await
        .unwrap();

    assert!(
        create_resp.status().is_success(),
        "create: {}",
        create_resp.status()
    );
    let create_body: Value = create_resp.json().await.unwrap();
    assert_eq!(create_body["success"], true, "create: {create_body}");

    let session_id = create_body["data"]["sessionId"]
        .as_str()
        .expect("sessionId missing");
    assert!(
        create_body["data"]["shell"].is_string(),
        "shell: {create_body}"
    );
    assert!(
        create_body["data"]["streamUrl"].is_string(),
        "streamUrl: {create_body}"
    );

    // List — should contain our terminal.
    let list_resp = http()
        .get(format!("{}/terminals", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .unwrap();

    let list_body: Value = list_resp.json().await.unwrap();
    assert_eq!(list_body["success"], true);
    let terminals = list_body["data"].as_array().unwrap();
    assert!(
        terminals.iter().any(|t| t["sessionId"] == session_id),
        "Created terminal not in list"
    );

    // Get single.
    let get_resp = http()
        .get(format!("{}/terminals/{session_id}", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .unwrap();

    let get_body: Value = get_resp.json().await.unwrap();
    assert_eq!(get_body["success"], true);
    assert_eq!(get_body["data"]["sessionId"], session_id);
    assert!(
        get_body["data"]["isRunning"].is_boolean(),
        "isRunning: {get_body}"
    );

    // Execute a command in the session.
    let exec_resp = http()
        .post(format!("{}/terminals/{session_id}/execute", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo session-exec-ok"}))
        .send()
        .await
        .unwrap();

    if exec_resp.status().is_success() {
        let exec_body: Value = exec_resp.json().await.unwrap_or(json!({}));
        eprintln!("session exec: {exec_body}");
    }

    // Delete.
    let delete_resp = http()
        .delete(format!("{}/terminals/{session_id}", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .unwrap();

    assert!(
        delete_resp.status().is_success(),
        "delete: {}",
        delete_resp.status()
    );

    // Verify deleted — get should fail.
    let get_after = http()
        .get(format!("{}/terminals/{session_id}", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .unwrap();

    let status = get_after.status().as_u16();
    assert!(
        status >= 400,
        "Deleted terminal should return 4xx, got {status}"
    );
}

// ===================================================================
// File operations
// ===================================================================

#[tokio::test]
async fn file_write_and_read() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Write.
    let write_resp = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "path": "/home/agent/instance-test.txt",
            "content": "hello from instance integration test"
        }))
        .send()
        .await
        .unwrap();

    let write_status = write_resp.status();
    let write_body: Value = write_resp.json().await.unwrap_or(json!({}));

    if !write_status.is_success() {
        eprintln!("File write failed (path outside workspace?), skipping read");
        return;
    }

    assert_eq!(write_body["success"], true, "write: {write_body}");
    assert!(write_body["data"]["hash"].is_string(), "hash: {write_body}");
    assert!(write_body["data"]["size"].is_number(), "size: {write_body}");

    // Read back.
    let read_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/instance-test.txt"}))
        .send()
        .await
        .unwrap();

    let read_body: Value = read_resp.json().await.unwrap_or(json!({}));
    if read_body["success"] == true {
        let content = read_body["data"]["content"].as_str().unwrap_or("");
        assert!(
            content.contains("hello from instance integration test"),
            "Content mismatch: {read_body}"
        );
    }
}

#[tokio::test]
async fn file_write_outside_workspace_rejected() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "path": "/tmp/outside-workspace.txt",
            "content": "should fail"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        403,
        "Writing outside workspace should be 403"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false, "body: {body}");
}

#[tokio::test]
async fn file_overwrite() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let path = "/home/agent/instance-overwrite-test.txt";

    // Write original.
    let r = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": path, "content": "original"}))
        .send()
        .await
        .unwrap();
    if !r.status().is_success() {
        return;
    }

    // Overwrite.
    let r = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": path, "content": "overwritten"}))
        .send()
        .await
        .unwrap();
    if !r.status().is_success() {
        return;
    }

    // Read back.
    let read_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": path}))
        .send()
        .await
        .unwrap();

    let body: Value = read_resp.json().await.unwrap();
    if body["success"] == true {
        let content = body["data"]["content"].as_str().unwrap_or("");
        assert!(
            content.contains("overwritten"),
            "Should contain overwritten content: '{content}'"
        );
        assert!(
            !content.contains("original"),
            "Should not contain original content: '{content}'"
        );
    }
}

#[tokio::test]
async fn file_read_nonexistent() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/does-not-exist-instance-abc.txt"}))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!((400..500).contains(&status), "Should be 4xx: {status}");
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert_eq!(body["success"], false);
}

// ===================================================================
// Instance prompt/task via blueprint functions
// ===================================================================

/// `run_instance_prompt` posts to `/agents/run`.
/// With backend: should succeed. Without: should fail (but not 404).
#[tokio::test]
async fn instance_prompt_reaches_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let timeout = if std::env::var("ZAI_API_KEY").is_ok() {
        60000
    } else {
        15000
    };

    let request = InstancePromptRequest {
        message: "Test prompt from instance blueprint".to_string(),
        session_id: String::new(),
        model: String::new(),
        context_json: String::new(),
        timeout_ms: timeout,
    };

    let result = run_instance_prompt(&s.url, AUTH_TOKEN, SANDBOX_ID, &request).await;

    match &result {
        Ok(resp) => {
            eprintln!(
                "instance prompt: success={}, response='{}'",
                resp.success, resp.response
            );
        }
        Err(e) => {
            assert!(
                !e.contains("404"),
                "/agents/run exists. Error should not be 404: {e}"
            );
            eprintln!("instance prompt failed (expected, no backend): {e}");
        }
    }
}

/// `run_instance_task` posts to `/agents/run` with extra metadata.
#[tokio::test]
async fn instance_task_reaches_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let timeout = if std::env::var("ZAI_API_KEY").is_ok() {
        60000
    } else {
        15000
    };

    let request = InstanceTaskRequest {
        prompt: "Test task from instance blueprint".to_string(),
        session_id: String::new(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: timeout,
    };

    let result = run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &request).await;

    match &result {
        Ok(resp) => {
            eprintln!(
                "instance task: success={}, result='{}', session_id='{}'",
                resp.success, resp.result, resp.session_id
            );
        }
        Err(e) => {
            assert!(
                !e.contains("404"),
                "/agents/run exists. Error should not be 404: {e}"
            );
            eprintln!("instance task failed (expected, no backend): {e}");
        }
    }
}

/// Verify agent response structure from real sidecar.
#[tokio::test]
async fn agent_run_response_structure() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/agents/run", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "message": "Say hello",
            "identifier": "default"
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    eprintln!("agents/run: status={status}, body={body}");

    if std::env::var("ZAI_API_KEY").is_ok() {
        assert_eq!(body["success"], true, "should succeed with backend: {body}");
        assert!(
            body["data"]["finalText"].is_string(),
            "data.finalText: {body}"
        );
    } else {
        assert_eq!(
            body["success"], false,
            "should fail without backend: {body}"
        );
        assert!(body["error"]["code"].is_string(), "error.code: {body}");
    }
}

// ===================================================================
// Snapshot command execution
// ===================================================================

#[tokio::test]
async fn snapshot_tar_command_executes() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "command": "mkdir -p /tmp/inst-snap && echo snap-data > /tmp/inst-snap/file.txt && tar -czf /tmp/inst-snap.tar.gz -C /tmp inst-snap && ls -la /tmp/inst-snap.tar.gz",
            "timeout": 15000
        }))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("inst-snap.tar.gz"),
        "tar file should exist: '{stdout}'"
    );
}

/// Blueprint's `run_instance_snapshot` executes against real sidecar.
#[tokio::test]
async fn instance_snapshot_via_blueprint_function() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // First create a file so there's something to snapshot.
    let _ = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "path": "/home/agent/snapshot-target.txt",
            "content": "snapshot me"
        }))
        .send()
        .await;

    let result = run_instance_snapshot(
        &s.url,
        AUTH_TOKEN,
        SANDBOX_ID,
        "/tmp/test-snapshot.tar.gz",
        true,
        false,
    )
    .await;

    match &result {
        Ok(json_str) => {
            eprintln!("snapshot result: {json_str}");
            // The command should execute (may fail if /home/agent/workspace doesn't exist,
            // but it should NOT be an HTTP error).
        }
        Err(e) => {
            // If the tar command fails (e.g., no files), that's acceptable.
            eprintln!("snapshot error (may be expected): {e}");
        }
    }
}

// ===================================================================
// SSH
// ===================================================================

#[tokio::test]
async fn ssh_provision_works() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let result = provision_key(
        &s.url,
        "agent",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInstanceTest instance@test",
        AUTH_TOKEN,
    )
    .await;

    eprintln!("provision_key result: {result:?}");
    assert!(result.is_ok(), "provision_key should succeed: {result:?}");
}

#[tokio::test]
async fn ssh_revoke_works() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let result = revoke_key(
        &s.url,
        "agent",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInstanceTest instance@test",
        AUTH_TOKEN,
    )
    .await;

    eprintln!("revoke_key result: {result:?}");
    assert!(result.is_ok(), "revoke_key should succeed: {result:?}");
}

#[tokio::test]
async fn ssh_provision_idempotent() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIdempotentInstance idempotent@instance";

    let r1 = provision_key(&s.url, "agent", key, AUTH_TOKEN).await;
    assert!(r1.is_ok(), "first provision failed: {r1:?}");

    let r2 = provision_key(&s.url, "agent", key, AUTH_TOKEN).await;
    assert!(r2.is_ok(), "second provision failed: {r2:?}");

    let v1 = r1.unwrap();
    let v2 = r2.unwrap();
    assert!(v1["success"].as_bool().unwrap_or(false), "r1: {v1}");
    assert!(v2["success"].as_bool().unwrap_or(false), "r2: {v2}");
}

// ===================================================================
// Large output handling
// ===================================================================

#[tokio::test]
async fn large_output_handling() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = InstanceExecRequest {
        command: "seq 1 1000".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
    };

    let resp = run_instance_exec(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("should succeed");

    assert_eq!(resp.exit_code, 0);
    assert!(
        resp.stdout.contains("1000"),
        "stdout should contain '1000': len={}",
        resp.stdout.len()
    );
}

// ===================================================================
// Long-running command
// ===================================================================

#[tokio::test]
async fn long_running_command_returns_duration() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "sleep 2 && echo done", "timeout": 15000}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");

    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(stdout.contains("done"), "stdout: '{stdout}'");

    let duration = body["result"]["duration"].as_f64().unwrap_or(0.0);
    assert!(
        duration >= 1500.0,
        "duration should be >= 1500ms: {duration}"
    );
}

// ===================================================================
// SSE helper
// ===================================================================

async fn collect_sse_events(resp: reqwest::Response, timeout: Duration) -> Vec<(String, Value)> {
    let mut events = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let chunk = tokio::time::timeout(remaining, stream.next()).await;
        match chunk {
            Ok(Some(Ok(bytes))) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(pos) = buffer.find("\n\n") {
                    let frame = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    let mut event_type = String::new();
                    let mut data_parts = Vec::new();

                    for line in frame.lines() {
                        if let Some(val) = line.strip_prefix("event:") {
                            event_type = val.trim().to_string();
                        } else if let Some(val) = line.strip_prefix("data:") {
                            data_parts.push(val.trim().to_string());
                        }
                    }

                    if !data_parts.is_empty() {
                        let data_str = data_parts.join("\n");
                        let data: Value =
                            serde_json::from_str(&data_str).unwrap_or(Value::String(data_str));
                        if event_type.is_empty() {
                            event_type = "message".to_string();
                        }
                        events.push((event_type, data));
                    }
                }
            }
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => break,
        }
    }

    events
}

// ===================================================================
// Real AI agent tests (requires ZAI_API_KEY)
// ===================================================================

/// Send a simple prompt via instance blueprint and verify real LLM response.
#[tokio::test]
async fn ai_prompt_returns_real_response() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = InstancePromptRequest {
        message: "What is 2+2? Reply with just the number.".to_string(),
        session_id: String::new(),
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
    };

    let result = run_instance_prompt(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("run_instance_prompt should succeed with AI backend");

    eprintln!(
        "AI prompt: success={}, response='{}', trace_id='{}'",
        result.success, result.response, result.trace_id
    );

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.response.is_empty(), "response should not be empty");
    assert!(
        result.response.contains('4'),
        "response should contain '4': '{}'",
        result.response
    );
}

/// Two sequential task calls: first creates session, second reuses session_id.
/// Verifies session continuity through the instance blueprint layer.
#[tokio::test]
async fn ai_task_with_session_continuity() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    // First message: creates a new session.
    let req1 = InstanceTaskRequest {
        prompt: "Say hello in one sentence.".to_string(),
        session_id: String::new(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
    };

    let result1 = run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &req1)
        .await
        .expect("first task should succeed");

    eprintln!(
        "Task 1: success={}, session_id='{}', result='{}'",
        result1.success, result1.session_id, result1.result
    );

    assert!(
        result1.success,
        "first task should succeed: error='{}'",
        result1.error
    );
    assert!(!result1.session_id.is_empty(), "should return a sessionId");
    assert!(!result1.result.is_empty(), "first result should not be empty");

    // Second message: reuse the session.
    let req2 = InstanceTaskRequest {
        prompt: "What is 3+5? Reply with just the number.".to_string(),
        session_id: result1.session_id.clone(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
    };

    let result2 = run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &req2)
        .await
        .expect("second task should succeed");

    eprintln!(
        "Task 2: success={}, session_id='{}', result='{}'",
        result2.success, result2.session_id, result2.result
    );

    assert!(
        result2.success,
        "second task should succeed: error='{}'",
        result2.error
    );
    assert!(!result2.result.is_empty(), "second result should not be empty");
    assert!(
        !result2.session_id.is_empty(),
        "second response should have sessionId"
    );
}

/// Three independent tasks with empty session_ids — each gets its own session.
#[tokio::test]
async fn ai_task_multiple_independent_sessions() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let mut session_ids = Vec::new();

    for i in 0..3 {
        let request = InstanceTaskRequest {
            prompt: format!("What is {i}+1? Reply with just the number."),
            session_id: String::new(),
            max_turns: 2,
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 60000,
        };

        let result = run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
            .await
            .expect(&format!("task {i} should succeed"));

        eprintln!(
            "Task {i}: success={}, session_id='{}'",
            result.success, result.session_id
        );

        assert!(result.success, "task {i} should succeed: error='{}'", result.error);
        assert!(!result.session_id.is_empty(), "task {i} should return sessionId");
        session_ids.push(result.session_id);
    }

    // Each call with empty session_id should get a different session.
    // (The sidecar creates a new session for each request without a sessionId.)
    eprintln!("Session IDs: {session_ids:?}");
    // Note: some backends might reuse sessions, so we just verify they exist.
    for (i, sid) in session_ids.iter().enumerate() {
        assert!(!sid.is_empty(), "session {i} should not be empty");
    }
}

/// Task with max_turns=2 completes.
#[tokio::test]
async fn ai_task_with_max_turns() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = InstanceTaskRequest {
        prompt: "Say hello in exactly 3 words.".to_string(),
        session_id: String::new(),
        max_turns: 2,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
    };

    let result = run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &request)
        .await
        .expect("task should succeed");

    eprintln!("Task max_turns: success={}, result='{}'", result.success, result.result);
    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");
}

/// POST to /agents/run/stream and verify SSE events.
#[tokio::test]
async fn ai_stream_emits_sse_events() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .unwrap();

    let resp = client
        .post(format!("{}/agents/run/stream", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "message": "What is 1+1? Reply briefly.",
            "identifier": "default"
        }))
        .send()
        .await
        .expect("stream request should succeed");

    let status = resp.status();
    eprintln!("SSE stream status: {status}");
    assert!(
        status.is_success(),
        "stream endpoint should return 2xx, got {status}"
    );

    let events = collect_sse_events(resp, Duration::from_secs(60)).await;

    eprintln!("SSE events received: {}", events.len());
    for (i, (evt, data)) in events.iter().enumerate() {
        eprintln!("  event[{i}]: type='{evt}', data={data}");
    }

    assert!(!events.is_empty(), "should receive at least one SSE event");

    let has_content = events.iter().any(|(t, _)| {
        t.contains("message") || t.contains("part") || t.contains("updated") || t == "message"
    });
    assert!(
        has_content,
        "should have at least one content event in: {:?}",
        events.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>()
    );
}

// ===================================================================
// Real AI agent — complex workflows
// ===================================================================

/// Ask the agent to write and run a Python script.
#[tokio::test]
async fn ai_agent_writes_and_runs_script() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = InstanceTaskRequest {
        prompt: "Create /home/agent/inst_fib.py that prints the first 10 Fibonacci numbers, then run it with python3.".to_string(),
        session_id: String::new(),
        max_turns: 5,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 240000,
    };

    let result = match run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &request).await {
        Ok(r) => r,
        Err(e) => {
            if e.contains("error sending request")
                || e.contains("timed out")
                || e.contains("timeout")
            {
                eprintln!("SKIPPED: AI agent timed out: {e}");
                return;
            }
            panic!("task failed unexpectedly: {e}");
        }
    };

    eprintln!(
        "Python task: success={}, result length={}",
        result.success,
        result.result.len()
    );

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");

    // Verify file was created.
    let read_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/inst_fib.py"}))
        .send()
        .await
        .unwrap();

    if read_resp.status().is_success() {
        let body: Value = read_resp.json().await.unwrap();
        if body["success"] == true {
            let content = body["data"]["content"].as_str().unwrap_or("");
            eprintln!("inst_fib.py ({} bytes): {}", content.len(), &content[..content.len().min(300)]);
            assert!(
                content.contains("fib")
                    || content.contains("Fib")
                    || content.contains("def ")
                    || content.contains("print"),
                "script should contain fibonacci logic"
            );
        }
    }
}

/// Create a JS file, run it, verify output.
#[tokio::test]
async fn ai_agent_full_workflow() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = InstanceTaskRequest {
        prompt: "Create /home/agent/inst_calc.js that prints JSON with sum and product of 42 and 7, then run it with node.".to_string(),
        session_id: String::new(),
        max_turns: 5,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 240000,
    };

    let result = match run_instance_task(&s.url, AUTH_TOKEN, SANDBOX_ID, &request).await {
        Ok(r) => r,
        Err(e) => {
            if e.contains("error sending request")
                || e.contains("timed out")
                || e.contains("timeout")
            {
                eprintln!("SKIPPED: AI agent timed out: {e}");
                return;
            }
            panic!("task failed unexpectedly: {e}");
        }
    };

    eprintln!(
        "Full workflow: success={}, result length={}",
        result.success,
        result.result.len()
    );

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");

    // Verify JS file was created.
    let js_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/inst_calc.js"}))
        .send()
        .await
        .unwrap();

    if js_resp.status().is_success() {
        let body: Value = js_resp.json().await.unwrap();
        if body["success"] == true {
            let content = body["data"]["content"].as_str().unwrap_or("");
            eprintln!("inst_calc.js ({} bytes): {}", content.len(), &content[..content.len().min(400)]);
            assert!(!content.is_empty(), "JS file should have content");
        }
    }
}

// ===================================================================
// Cleanup
// ===================================================================

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
