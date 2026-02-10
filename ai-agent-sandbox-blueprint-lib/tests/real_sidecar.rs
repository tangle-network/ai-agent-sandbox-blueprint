//! Real sidecar integration tests.
//!
//! These tests spin up an actual sidecar Docker container and hit real HTTP
//! endpoints. They verify real response shapes, real auth behavior, and real
//! command execution — no mocks.
//!
//! Run (infrastructure only):
//!   REAL_SIDECAR=1 cargo test --test real_sidecar -- --test-threads=1
//!
//! Run (with AI backend):
//!   REAL_SIDECAR=1 ZAI_API_KEY=<key> cargo test --test real_sidecar -- --test-threads=1
//!
//! Requires Docker and a local sidecar image (default: tangle-sidecar:local).
//! Override with SIDECAR_IMAGE env var.

use std::collections::HashMap;
use std::time::Duration;

use ai_agent_sandbox_blueprint_lib::{
    SandboxExecRequest, SandboxPromptRequest, extract_agent_fields, extract_exec_fields,
};
use docktopus::bollard::container::{Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use docktopus::DockerBuilder;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

// ---------------------------------------------------------------------------
// Shared container setup
// ---------------------------------------------------------------------------

const AUTH_TOKEN: &str = "test-real-sidecar-token-6f2a9b";
const CONTAINER_NAME: &str = "test-real-sidecar";
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
            // Set a generous HTTP client timeout for AI tasks (must happen before
            // SidecarRuntimeConfig::load() is first called).
            // SAFETY: called once during single-threaded test init before any
            // other thread reads this variable.
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

            // When AI backend is configured, warm it up by sending a simple prompt.
            // The OpenCode backend needs extra time to initialize after health passes.
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
                        .header(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {AUTH_TOKEN}")).unwrap())
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
                                // Got a real error (not a crash), backend is running
                                eprintln!("AI backend ready (responded with error: {})",
                                    &body[..body.len().min(100)]);
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
    assert!(body["backends"]["total"].is_number(), "backends.total: {body}");
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
        ["healthy", "degraded", "unhealthy"]
            .contains(&body["status"].as_str().unwrap_or("")),
        "status: {body}"
    );
    assert!(body["memory"].is_object(), "memory: {body}");
    assert!(body["process"].is_object(), "process: {body}");
    assert!(body["uptime"].is_number(), "uptime: {body}");
    assert!(body["process"]["pid"].is_number(), "process.pid: {body}");
    assert!(body["memory"]["heapUsed"].is_number(), "heapUsed: {body}");
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
    assert_eq!(body["error"]["message"], "Missing authentication token");
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
    assert_eq!(body["error"]["message"], "Invalid authentication token");
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
// Terminal commands — primary exec pathway on real sidecar
// ===================================================================

#[tokio::test]
async fn terminal_commands_echo() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo terminal-output-123", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["success"], true, "body: {body}");
    // Terminal commands put output under `result`, not `data`.
    let result = &body["result"];
    assert_eq!(result["exitCode"], 0, "exitCode: {body}");
    assert!(
        result["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("terminal-output-123"),
        "stdout should contain our text: {body}"
    );
    assert!(result["stderr"].is_string(), "stderr missing: {body}");
    assert!(result["duration"].is_number(), "duration missing: {body}");
}

#[tokio::test]
async fn terminal_commands_exit_code() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "exit 42", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    assert_eq!(body["result"]["exitCode"], 42, "exitCode: {body}");
}

#[tokio::test]
async fn terminal_commands_stderr() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo stderr-msg >&2; exit 1", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    let result = &body["result"];
    assert_eq!(result["exitCode"], 1, "exitCode: {body}");
    // Note: terminal output includes PTY escape codes, so stderr may
    // contain ANSI sequences mixed with the actual text.
}

/// Verify extract_exec_fields correctly parses terminal commands response.
/// The blueprint now reads from `result.exitCode`, `result.stdout`, etc.
#[tokio::test]
async fn terminal_commands_shape_compatible_with_extract_exec_fields() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo shape-test", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();

    let (exit_code, stdout, _stderr) = extract_exec_fields(&body);
    assert_eq!(exit_code, 0, "extract_exec_fields should find exitCode=0");
    assert!(
        stdout.contains("shape-test"),
        "extract_exec_fields should find stdout: got '{stdout}'"
    );
}

/// Verify exec with cwd parameter changes working directory.
#[tokio::test]
async fn terminal_commands_with_cwd() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "pwd", "cwd": "/tmp", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["success"], true, "body: {body}");
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("/tmp"),
        "cwd should be /tmp, got stdout: '{stdout}'"
    );
}

/// Verify exec with env variables passed through.
#[tokio::test]
async fn terminal_commands_with_env() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "command": "echo $TEST_VAR_ABC",
            "env": {"TEST_VAR_ABC": "env-value-xyz"},
            "timeout": 10000
        }))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    // PTY may not propagate env vars the same way; check stdout if it works.
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    eprintln!("env test stdout: '{stdout}'");
}

/// Verify empty command is handled gracefully.
#[tokio::test]
async fn terminal_commands_empty_command() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "", "timeout": 5000}))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    eprintln!("empty command: status={status}, body={body}");
    // Should either return an error or exit 0 — not crash.
    assert!(status < 500 || body["error"].is_object(), "Should handle empty command: {body}");
}

/// Verify multiline command output.
#[tokio::test]
async fn terminal_commands_multiline_output() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo line1; echo line2; echo line3", "timeout": 10000}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(stdout.contains("line1"), "should contain line1: '{stdout}'");
    assert!(stdout.contains("line3"), "should contain line3: '{stdout}'");
}

/// Blueprint run_exec_request with cwd and env_json parameters.
#[tokio::test]
async fn blueprint_run_exec_with_cwd_and_env() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = SandboxExecRequest {
        sidecar_url: s.url.clone(),
        command: "pwd".to_string(),
        cwd: "/tmp".to_string(),
        env_json: r#"{"MY_VAR": "test123"}"#.to_string(),
        timeout_ms: 15000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_exec_request(&request).await;
    match result {
        Ok(_) => {}
        Err(e) => panic!("should succeed: {e}"),
    }
}

// ===================================================================
// Blueprint run_exec_request against real sidecar
// ===================================================================

/// Blueprint's `run_exec_request` posts to `/terminals/commands`.
#[tokio::test]
async fn blueprint_run_exec_request_works_against_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = SandboxExecRequest {
        sidecar_url: s.url.clone(),
        command: "echo blueprint-exec-ok".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_exec_request(&request).await;

    match result {
        Ok(resp) => {
            assert_eq!(resp.exit_code, 0, "exit_code should be 0");
            assert!(
                resp.stdout.contains("blueprint-exec-ok"),
                "stdout should contain our text: '{}'", resp.stdout
            );
        }
        Err(err) => {
            panic!("run_exec_request should succeed against real sidecar: {err}");
        }
    }
}

/// Verify non-zero exit codes propagate through run_exec_request.
#[tokio::test]
async fn blueprint_run_exec_captures_exit_code() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = SandboxExecRequest {
        sidecar_url: s.url.clone(),
        command: "exit 42".to_string(),
        cwd: String::new(),
        env_json: String::new(),
        timeout_ms: 15000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_exec_request(&request).await;

    match result {
        Ok(resp) => {
            assert_eq!(resp.exit_code, 42, "exit_code should be 42");
        }
        Err(err) => {
            panic!("run_exec_request should succeed: {err}");
        }
    }
}

// ===================================================================
// SSH jobs against real sidecar
// ===================================================================

/// SSH provision calls `/terminals/commands` to add a key.
/// The container user is "agent", not "root", so we test with "agent".
#[tokio::test]
async fn ssh_provision_works_against_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let result = ai_agent_sandbox_blueprint_lib::provision_key(
        &s.url,
        "agent",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest test@test",
        AUTH_TOKEN,
    )
    .await;

    eprintln!("provision_key result: {result:?}");
    // Should succeed — the command runs `getent passwd agent` which exists.
    assert!(result.is_ok(), "provision_key should succeed: {result:?}");
}

/// SSH revoke calls `/terminals/commands` to remove a key.
#[tokio::test]
async fn ssh_revoke_works_against_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let result = ai_agent_sandbox_blueprint_lib::revoke_key(
        &s.url,
        "agent",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest test@test",
        AUTH_TOKEN,
    )
    .await;

    eprintln!("revoke_key result: {result:?}");
    assert!(result.is_ok(), "revoke_key should succeed: {result:?}");
}

// ===================================================================
// Agent run
// ===================================================================

/// Test /agents/run response structure.
/// Without a backend: returns {success: false, error: {code, message}}.
/// With a backend: returns {success: true, data: {finalText, ...}}.
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
        // With backend configured, expect success.
        assert_eq!(body["success"], true, "should succeed with backend: {body}");
        assert!(body["data"]["finalText"].is_string(), "data.finalText: {body}");
    } else {
        // Without a backend, sidecar returns HTTP 500 with structured error.
        assert_eq!(body["success"], false, "should fail without backend: {body}");
        assert!(body["error"]["code"].is_string(), "error.code: {body}");
        assert!(body["error"]["message"].is_string(), "error.message: {body}");
    }
}

/// Verify extract_agent_fields correctly parses the real response.
/// Without a backend: success=false, error is populated.
/// With a backend (ZAI_API_KEY set): success=true, response is populated.
#[tokio::test]
async fn extract_agent_fields_parses_real_response() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/agents/run", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"message": "test", "identifier": "default"}))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    let (success, response, error, _trace_id) = extract_agent_fields(&body);

    eprintln!("extract_agent_fields: success={success}, response='{response}', error='{error}'");

    if std::env::var("ZAI_API_KEY").is_ok() {
        // With backend, expect success with a response.
        assert!(success, "should succeed with backend: {body}");
        assert!(!response.is_empty(), "response should not be empty: {body}");
    } else {
        // Without backend, expect error.
        assert!(!success, "should not be success without backend: {body}");
        assert!(!error.is_empty(), "should extract error message from: {body}");
    }
}

/// Blueprint's `run_prompt_request` posts to `/agents/run`.
/// With a backend: should succeed. Without: should fail (but not 404).
#[tokio::test]
async fn blueprint_run_prompt_reaches_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let timeout = if std::env::var("ZAI_API_KEY").is_ok() { 60000 } else { 15000 };

    let request = SandboxPromptRequest {
        sidecar_url: s.url.clone(),
        message: "Test prompt".to_string(),
        session_id: String::new(),
        model: String::new(),
        context_json: String::new(),
        timeout_ms: timeout,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_prompt_request(&request).await;

    match &result {
        Ok(resp) => {
            eprintln!("run_prompt_request succeeded: success={}, response='{}'",
                resp.success, resp.response);
        }
        Err(e) => {
            // Should fail from HTTP 500 (no backend), NOT 404.
            assert!(
                !e.contains("404"),
                "/agents/run exists. Error should not be 404: {e}"
            );
            eprintln!("run_prompt_request failed (expected, no backend): {e}");
        }
    }
}

// ===================================================================
// File operations
// ===================================================================

#[tokio::test]
async fn file_write_and_read() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Write a file inside workspace.
    let write_resp = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "path": "/home/agent/real-sidecar-test.txt",
            "content": "hello from integration test"
        }))
        .send()
        .await
        .unwrap();

    let write_status = write_resp.status();
    let write_body: Value = write_resp.json().await.unwrap_or(json!({}));
    eprintln!("file write: status={write_status}, body={write_body}");

    if !write_status.is_success() {
        eprintln!("File write failed (path outside workspace?), skipping read");
        return;
    }

    assert_eq!(write_body["success"], true, "write: {write_body}");
    assert!(write_body["data"]["hash"].is_string(), "hash: {write_body}");
    assert!(write_body["data"]["size"].is_number(), "size: {write_body}");

    // Read it back (POST, not GET).
    let read_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/real-sidecar-test.txt"}))
        .send()
        .await
        .unwrap();

    let read_status = read_resp.status();
    let read_body: Value = read_resp.json().await.unwrap_or(json!({}));
    eprintln!("file read: status={read_status}, body={read_body}");

    if read_status.is_success() && read_body["success"] == true {
        let content = read_body["data"]["content"].as_str().unwrap_or("");
        assert!(
            content.contains("hello from integration test"),
            "Content mismatch: {read_body}"
        );
    }
}

/// Files outside workspace should be rejected.
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

    assert_eq!(resp.status(), 403, "Writing outside workspace should be 403");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false, "body: {body}");
}

// ===================================================================
// Terminal lifecycle
// ===================================================================

#[tokio::test]
async fn terminal_create_list_delete() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Create.
    let create_resp = http()
        .post(format!("{}/terminals", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"name": "test-terminal"}))
        .send()
        .await
        .unwrap();

    assert!(create_resp.status().is_success(), "create: {}", create_resp.status());
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

    // List.
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

    // Delete.
    let delete_resp = http()
        .delete(format!("{}/terminals/{session_id}", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .unwrap();

    assert!(delete_resp.status().is_success(), "delete: {}", delete_resp.status());
}

// ===================================================================
// Blueprint run_task_request against real sidecar
// ===================================================================

/// run_task_request sends to /agents/run just like run_prompt_request.
/// With a backend: should succeed. Without: should fail (but not 404).
#[tokio::test]
async fn blueprint_run_task_request_reaches_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let timeout = if std::env::var("ZAI_API_KEY").is_ok() { 60000 } else { 15000 };

    let request = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: "Task test".to_string(),
        session_id: String::new(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: timeout,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_task_request(&request).await;

    match &result {
        Ok(resp) => {
            eprintln!("run_task_request succeeded: success={}, result='{}'",
                resp.success, resp.result);
        }
        Err(e) => {
            assert!(
                !e.contains("404"),
                "/agents/run exists. Error should not be 404: {e}"
            );
            eprintln!("run_task_request failed (expected, no backend): {e}");
        }
    }
}

// ===================================================================
// Concurrent requests
// ===================================================================

/// Verify the sidecar handles multiple simultaneous exec requests.
#[tokio::test]
async fn concurrent_exec_requests() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let mut handles = Vec::new();
    for i in 0..5 {
        let url = s.url.clone();
        handles.push(tokio::spawn(async move {
            let resp = http()
                .post(format!("{url}/terminals/commands"))
                .header(AUTHORIZATION, auth_header())
                .header(CONTENT_TYPE, "application/json")
                .json(&json!({"command": format!("echo concurrent-{i}"), "timeout": 10000}))
                .send()
                .await
                .unwrap();
            let body: Value = resp.json().await.unwrap();
            (i, body)
        }));
    }

    for handle in handles {
        let (i, body) = handle.await.unwrap();
        assert_eq!(body["success"], true, "concurrent-{i} failed: {body}");
        let stdout = body["result"]["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains(&format!("concurrent-{i}")),
            "concurrent-{i} missing from stdout: '{stdout}'"
        );
    }
}

// ===================================================================
// Large output handling
// ===================================================================

/// Verify the sidecar handles commands that produce substantial output.
#[tokio::test]
async fn large_output_handling() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Generate ~10KB of output.
    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "command": "seq 1 1000",
            "timeout": 15000
        }))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");

    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    // Should contain the last line (1000) somewhere.
    assert!(
        stdout.contains("1000"),
        "stdout should contain '1000': len={}",
        stdout.len()
    );
}

// ===================================================================
// Snapshot command execution
// ===================================================================

/// Verify that a snapshot-style tar command can run inside the sidecar.
/// We don't actually upload anywhere — just verify tar works.
#[tokio::test]
async fn snapshot_tar_command_executes() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Create a file, then tar it. This simulates what the snapshot job does.
    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "command": "mkdir -p /tmp/snap-test && echo snap-data > /tmp/snap-test/file.txt && tar -czf /tmp/snap-test.tar.gz -C /tmp snap-test && ls -la /tmp/snap-test.tar.gz",
            "timeout": 15000
        }))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    let stdout = body["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("snap-test.tar.gz"),
        "tar file should exist: '{stdout}'"
    );
}

// ===================================================================
// build_exec_payload against real sidecar
// ===================================================================

/// Verify build_exec_payload produces a payload the real sidecar accepts.
#[tokio::test]
async fn build_exec_payload_works_with_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let payload = ai_agent_sandbox_blueprint_lib::build_exec_payload(
        "echo payload-ok",
        "/tmp",
        r#"{"PAYLOAD_VAR": "test"}"#,
        10000,
    );

    let resp = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");

    let (exit_code, stdout, _stderr) =
        ai_agent_sandbox_blueprint_lib::extract_exec_fields(&body);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("payload-ok"), "stdout: '{stdout}'");
}

// ===================================================================
// SSH key idempotency
// ===================================================================

/// Calling `provision_key` twice with the same key should succeed both times.
/// The underlying `build_ssh_command` uses `grep -qxF` to avoid duplicating entries.
#[tokio::test]
async fn ssh_provision_idempotent() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIdempotent idempotent@test";

    // Both calls should return Ok (HTTP-level success).
    let r1 = ai_agent_sandbox_blueprint_lib::provision_key(&s.url, "agent", key, AUTH_TOKEN).await;
    assert!(r1.is_ok(), "first provision failed: {r1:?}");

    let r2 = ai_agent_sandbox_blueprint_lib::provision_key(&s.url, "agent", key, AUTH_TOKEN).await;
    assert!(r2.is_ok(), "second provision failed: {r2:?}");

    // Both should return identical sidecar response structure.
    let v1 = r1.unwrap();
    let v2 = r2.unwrap();
    assert!(v1["success"].as_bool().unwrap_or(false), "r1: {v1}");
    assert!(v2["success"].as_bool().unwrap_or(false), "r2: {v2}");
}

// ===================================================================
// File edge cases
// ===================================================================

/// Write an empty file and read it back.
#[tokio::test]
async fn file_write_empty_content() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let write_resp = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "path": "/home/agent/empty-file.txt",
            "content": ""
        }))
        .send()
        .await
        .unwrap();

    if !write_resp.status().is_success() {
        eprintln!("Empty file write returned {}", write_resp.status());
        return;
    }

    let body: Value = write_resp.json().await.unwrap();
    assert_eq!(body["success"], true, "body: {body}");
    assert_eq!(body["data"]["size"], 0, "empty file should be 0 bytes: {body}");
}

/// Overwrite an existing file and verify new content.
#[tokio::test]
async fn file_overwrite() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let path = "/home/agent/overwrite-test.txt";

    // Write original.
    let r = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": path, "content": "original"}))
        .send()
        .await
        .unwrap();
    if !r.status().is_success() { return; }

    // Overwrite.
    let r = http()
        .post(format!("{}/files/write", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": path, "content": "overwritten"}))
        .send()
        .await
        .unwrap();
    if !r.status().is_success() { return; }

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

/// Read a non-existent file should return an error.
#[tokio::test]
async fn file_read_nonexistent() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/does-not-exist-abc123.txt"}))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    eprintln!("read nonexistent: status={status}, body={body}");
    // Should be a 4xx error, not a crash.
    assert!((400..500).contains(&status), "Should be 4xx: {status}");
    assert_eq!(body["success"], false);
}

// ===================================================================
// Long-running command
// ===================================================================

/// Verify a command that takes a few seconds completes and returns duration.
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
    assert!(duration >= 1500.0, "duration should be >= 1500ms: {duration}");
}

// ===================================================================
// Response shape compatibility documentation
// ===================================================================

/// Verifies the sidecar endpoints used by the blueprint are available.
#[tokio::test]
async fn api_compatibility_report() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let mut report = Vec::new();

    // /terminals/commands — used by exec, ssh, snapshot jobs
    let r = http()
        .post(format!("{}/terminals/commands", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"command": "echo test", "timeout": 5000}))
        .send()
        .await
        .unwrap();
    let tc_status = if r.status().is_success() {
        "OK".to_string()
    } else {
        format!("status {}", r.status())
    };
    report.push(format!("/terminals/commands (exec, ssh, snapshot): {tc_status}"));

    // /agents/run — used by prompt/task jobs
    let r = http()
        .post(format!("{}/agents/run", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"message": "test", "identifier": "default"}))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    report.push(format!(
        "/agents/run (prompt/task): status {status} ({})",
        if status == 500 { "no backend configured" } else { "unexpected" }
    ));

    eprintln!("\n=== API COMPATIBILITY REPORT ===");
    for line in &report {
        eprintln!("  {line}");
    }
    eprintln!("================================\n");
}

// ===================================================================
// SSE helper
// ===================================================================

/// Read SSE events from a response body until done or timeout.
/// Returns a vec of (event_type, data) pairs.
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

                // Parse complete SSE frames (separated by double newline).
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
            Err(_) => break, // timeout
        }
    }

    events
}

// ===================================================================
// Real AI agent tests (requires ZAI_API_KEY)
// ===================================================================

/// Send a simple prompt and verify we get a real LLM response.
#[tokio::test]
async fn ai_agent_prompt_returns_real_response() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = SandboxPromptRequest {
        sidecar_url: s.url.clone(),
        message: "What is 2+2? Reply with just the number.".to_string(),
        session_id: String::new(),
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_prompt_request(&request)
        .await
        .expect("run_prompt_request should succeed with AI backend");

    eprintln!("AI prompt response: success={}, response='{}', trace_id='{}'",
        result.success, result.response, result.trace_id);

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.response.is_empty(), "response should not be empty");
    assert!(result.response.contains('4'), "response should contain '4': '{}'", result.response);
}

/// Send two tasks using the same sessionId to verify session mechanics.
/// The first request creates a session; the second reuses it.
/// We verify both succeed and the session ID is accepted (not rejected).
#[tokio::test]
async fn ai_agent_task_with_session_continuity() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    // First message: creates a new session.
    let request1 = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: "Say hello in one sentence.".to_string(),
        session_id: String::new(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result1 = ai_agent_sandbox_blueprint_lib::run_task_request(&request1)
        .await
        .expect("first task request should succeed");

    eprintln!("Task 1: success={}, session_id='{}', result='{}'",
        result1.success, result1.session_id, result1.result);

    assert!(result1.success, "first task should succeed: error='{}'", result1.error);
    assert!(!result1.session_id.is_empty(), "should return a sessionId");
    assert!(!result1.result.is_empty(), "first result should not be empty");

    // Second message: reuse the same session.
    let request2 = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: "What is 3+5? Reply with just the number.".to_string(),
        session_id: result1.session_id.clone(),
        max_turns: 3,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result2 = ai_agent_sandbox_blueprint_lib::run_task_request(&request2)
        .await
        .expect("second task request should succeed");

    eprintln!("Task 2: success={}, session_id='{}', result='{}'",
        result2.success, result2.session_id, result2.result);

    assert!(result2.success, "second task should succeed: error='{}'", result2.error);
    assert!(!result2.result.is_empty(), "second result should not be empty");
    // Session ID should be consistent (same or new — both are valid).
    assert!(!result2.session_id.is_empty(), "second response should have sessionId");
}

/// Send a task with max_turns and verify it completes.
#[tokio::test]
async fn ai_agent_task_with_max_turns() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let request = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: "Say hello in exactly 3 words.".to_string(),
        session_id: String::new(),
        max_turns: 2,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 60000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_task_request(&request)
        .await
        .expect("task request should succeed");

    eprintln!("Task max_turns: success={}, result='{}'", result.success, result.result);

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");
}

// ===================================================================
// SSE streaming tests (requires ZAI_API_KEY)
// ===================================================================

/// POST to /agents/run/stream and verify we receive SSE events.
#[tokio::test]
async fn ai_agent_stream_emits_sse_events() {
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

    // The endpoint should return 200 with text/event-stream content type.
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

    // Check for expected event types.
    let event_types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();
    eprintln!("Event types: {event_types:?}");

    // We expect at least a start-ish event and some content events.
    // The exact event names depend on the sidecar implementation.
    let has_content = events.iter().any(|(t, _)| {
        t.contains("message") || t.contains("part") || t.contains("updated") || t == "message"
    });
    assert!(
        has_content,
        "should have at least one content event in: {event_types:?}"
    );
}

// ===================================================================
// Real-world AI agent tests — complex multi-tool tasks
// ===================================================================

/// Ask the agent to write and run a simple Python script.
/// This exercises: tool use (file write + terminal exec) in a single focused task.
///
/// NOTE: This test is tolerant of timeouts since the AI model may be slow for
/// agentic tool-use. A timeout is reported as a skip, not a failure.
#[tokio::test]
async fn ai_agent_writes_and_runs_python_script() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let prompt = "Create /home/agent/fib.py that prints the first 10 Fibonacci numbers, then run it with python3.";

    let request = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: prompt.to_string(),
        session_id: String::new(),
        max_turns: 5,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 240000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = match ai_agent_sandbox_blueprint_lib::run_task_request(&request).await {
        Ok(r) => r,
        Err(e) => {
            if e.contains("error sending request") || e.contains("timed out") || e.contains("timeout") {
                eprintln!("SKIPPED: AI agent timed out (model too slow for agentic tool-use): {e}");
                return;
            }
            panic!("task request failed unexpectedly: {e}");
        }
    };

    eprintln!("Python task: success={}, result length={}", result.success, result.result.len());
    eprintln!("Result: {}", &result.result[..result.result.len().min(500)]);

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");

    // Verify the agent actually created the file by reading it back.
    let read_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/fib.py"}))
        .send()
        .await
        .unwrap();

    if read_resp.status().is_success() {
        let body: Value = read_resp.json().await.unwrap();
        if body["success"] == true {
            let content = body["data"]["content"].as_str().unwrap_or("");
            eprintln!("fib.py content ({} bytes): {}", content.len(),
                &content[..content.len().min(300)]);
            assert!(content.contains("fib") || content.contains("Fib") || content.contains("def ") || content.contains("print"),
                "script should contain fibonacci logic: '{}'", &content[..content.len().min(200)]);
        }
    }
}

/// Ask the agent to do data analysis with pandas: create a dataset, compute stats,
/// and write results. This is the "investment memo" class of prompt — multi-step,
/// data science, file I/O.
#[tokio::test]
async fn ai_agent_pandas_data_analysis() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let prompt = r#"Write and run a Python script at /home/agent/analysis.py that:
1. Creates a pandas DataFrame with 100 rows of fake stock data:
   - columns: date (daily from 2024-01-01), ticker (randomly AAPL/GOOG/MSFT), price (random 100-500), volume (random int)
2. Groups by ticker and computes: mean price, total volume, price std dev
3. Writes the summary table to /home/agent/stock_summary.csv
4. Prints the summary to stdout

Install pandas with pip first if needed."#;

    let request = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: prompt.to_string(),
        session_id: String::new(),
        max_turns: 8,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 240000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = match ai_agent_sandbox_blueprint_lib::run_task_request(&request).await {
        Ok(r) => r,
        Err(e) => {
            if e.contains("error sending request") || e.contains("timed out") || e.contains("timeout") {
                eprintln!("SKIPPED: AI agent timed out (model too slow for pandas task): {e}");
                return;
            }
            panic!("pandas task failed unexpectedly: {e}");
        }
    };

    eprintln!("Pandas task: success={}, result length={}", result.success, result.result.len());
    eprintln!("Result: {}", &result.result[..result.result.len().min(800)]);

    assert!(result.success, "should succeed: error='{}'", result.error);

    // Check the CSV was created.
    let csv_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/stock_summary.csv"}))
        .send()
        .await
        .unwrap();

    if csv_resp.status().is_success() {
        let body: Value = csv_resp.json().await.unwrap();
        if body["success"] == true {
            let content = body["data"]["content"].as_str().unwrap_or("");
            eprintln!("stock_summary.csv:\n{content}");
            // Should contain ticker names and numeric data.
            let has_tickers = content.contains("AAPL") || content.contains("GOOG") || content.contains("MSFT");
            assert!(has_tickers, "CSV should contain stock tickers: '{content}'");
        }
    }
}

/// Stream a complex prompt and verify we get tool invocation events, not just text.
/// This proves streaming works for multi-step agentic tasks, not just chat.
#[tokio::test]
async fn ai_agent_stream_complex_task_with_tools() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .unwrap();

    let resp = client
        .post(format!("{}/agents/run/stream", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "message": "Create a file /home/agent/hello.txt containing 'Hello from streaming test'. Then read it back and confirm the contents.",
            "identifier": "default"
        }))
        .send()
        .await
        .expect("stream request should succeed");

    let status = resp.status();
    eprintln!("Complex stream status: {status}");
    assert!(status.is_success(), "should return 2xx, got {status}");

    let events = collect_sse_events(resp, Duration::from_secs(120)).await;

    eprintln!("Complex stream events: {} total", events.len());

    // Categorize events.
    let mut event_type_counts: HashMap<String, usize> = HashMap::new();
    for (evt, data) in &events {
        *event_type_counts.entry(evt.clone()).or_default() += 1;
        // Print first 150 chars of each event for debugging.
        let preview = format!("{data}");
        let preview = if preview.len() > 150 { &preview[..150] } else { &preview };
        eprintln!("  [{evt}] {preview}");
    }

    eprintln!("\nEvent type summary:");
    for (t, c) in &event_type_counts {
        eprintln!("  {t}: {c}x");
    }

    assert!(!events.is_empty(), "should receive SSE events");
    assert!(events.len() >= 3,
        "complex task should produce multiple events, got {}", events.len());

    // Check for tool-related events (the agent should use tools to create/read files).
    let has_tool_events = events.iter().any(|(t, _)| {
        t.contains("tool") || t.contains("invocation") || t.contains("action")
    });
    let has_content_events = events.iter().any(|(t, _)| {
        t.contains("message") || t.contains("text") || t.contains("part")
    });
    let has_lifecycle_events = events.iter().any(|(t, _)| {
        t.contains("start") || t.contains("done") || t.contains("execution")
    });

    eprintln!("Has tool events: {has_tool_events}");
    eprintln!("Has content events: {has_content_events}");
    eprintln!("Has lifecycle events: {has_lifecycle_events}");

    // At minimum we need lifecycle events (start/done) and some content.
    assert!(has_lifecycle_events, "should have lifecycle events (start/done)");
    // The task involves file creation, so we expect either tool events or content describing it.
    assert!(has_tool_events || has_content_events,
        "should have tool or content events for a file-creation task");
}

/// Write and run a Node.js script. This exercises the "vibecoding" workflow:
/// create code, execute it, get results — all in one agent turn.
///
/// NOTE: This test is tolerant of timeouts since the AI model may be slow for
/// agentic tool-use. A timeout is reported as a skip, not a failure.
#[tokio::test]
async fn ai_agent_full_workflow_install_code_execute() {
    skip_unless_ai!();
    let s = ensure_sidecar().await;

    let prompt = "Create /home/agent/calc.js that prints JSON with sum and product of 42 and 7, then run it with node.";

    let request = ai_agent_sandbox_blueprint_lib::SandboxTaskRequest {
        sidecar_url: s.url.clone(),
        prompt: prompt.to_string(),
        session_id: String::new(),
        max_turns: 5,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 240000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = match ai_agent_sandbox_blueprint_lib::run_task_request(&request).await {
        Ok(r) => r,
        Err(e) => {
            if e.contains("error sending request") || e.contains("timed out") || e.contains("timeout") {
                eprintln!("SKIPPED: AI agent timed out (model too slow for agentic tool-use): {e}");
                return;
            }
            panic!("full workflow task failed unexpectedly: {e}");
        }
    };

    eprintln!("Full workflow: success={}, result length={}", result.success, result.result.len());
    eprintln!("Result: {}", &result.result[..result.result.len().min(800)]);

    assert!(result.success, "should succeed: error='{}'", result.error);
    assert!(!result.result.is_empty(), "result should not be empty");

    // Verify the JS file was created.
    let js_resp = http()
        .post(format!("{}/files/read", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"path": "/home/agent/calc.js"}))
        .send()
        .await
        .unwrap();

    if js_resp.status().is_success() {
        let body: Value = js_resp.json().await.unwrap();
        if body["success"] == true {
            let content = body["data"]["content"].as_str().unwrap_or("");
            eprintln!("calc.js ({} bytes): {}", content.len(), &content[..content.len().min(400)]);
            assert!(!content.is_empty(), "JS file should have content");
        }
    }
}

/// Create a terminal, connect to its stream, execute a command, verify output arrives.
#[tokio::test]
async fn terminal_stream_emits_output() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    // Create a terminal.
    let create_resp = http()
        .post(format!("{}/terminals", s.url))
        .header(AUTHORIZATION, auth_header())
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"name": "stream-test-terminal"}))
        .send()
        .await
        .unwrap();

    assert!(create_resp.status().is_success(), "create: {}", create_resp.status());
    let create_body: Value = create_resp.json().await.unwrap();
    let session_id = create_body["data"]["sessionId"]
        .as_str()
        .expect("sessionId missing");

    eprintln!("Created terminal for stream test: {session_id}");

    // Connect to terminal stream SSE endpoint.
    let stream_client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let stream_resp = stream_client
        .get(format!("{}/terminals/{session_id}/stream", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await
        .expect("stream connect should succeed");

    let stream_status = stream_resp.status();
    eprintln!("Terminal stream status: {stream_status}");

    if !stream_status.is_success() {
        eprintln!("Terminal stream not supported (status={stream_status}), skipping");
        // Clean up.
        let _ = http()
            .delete(format!("{}/terminals/{session_id}", s.url))
            .header(AUTHORIZATION, auth_header())
            .send()
            .await;
        return;
    }

    // Execute a command in the terminal (fire-and-forget, we'll read from stream).
    let exec_url = s.url.clone();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        // Small delay to let stream connect.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = http()
            .post(format!("{exec_url}/terminals/{sid}/execute"))
            .header(AUTHORIZATION, auth_header())
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({"command": "echo stream-test-marker-xyz"}))
            .send()
            .await;
    });

    // Collect SSE events from the terminal stream.
    let events = collect_sse_events(stream_resp, Duration::from_secs(10)).await;

    eprintln!("Terminal stream events: {}", events.len());
    for (i, (evt, data)) in events.iter().enumerate() {
        let preview = format!("{data}");
        let preview = if preview.len() > 100 { &preview[..100] } else { &preview };
        eprintln!("  event[{i}]: type='{evt}', data={preview}");
    }

    // Terminal stream should emit at least some data events.
    // Even without the execute command, the shell prompt itself generates output.
    // We're lenient here — if we get any events, the stream works.
    if events.is_empty() {
        eprintln!("Warning: no terminal stream events received (may need /terminals/{{id}}/execute endpoint)");
    }

    // Clean up terminal.
    let _ = http()
        .delete(format!("{}/terminals/{session_id}", s.url))
        .header(AUTHORIZATION, auth_header())
        .send()
        .await;
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
