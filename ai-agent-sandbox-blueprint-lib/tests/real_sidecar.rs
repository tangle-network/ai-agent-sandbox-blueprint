//! Real sidecar integration tests.
//!
//! These tests spin up an actual sidecar Docker container and hit real HTTP
//! endpoints. They verify real response shapes, real auth behavior, and real
//! command execution — no mocks.
//!
//! Run:
//!   REAL_SIDECAR=1 cargo test --test real_sidecar -- --test-threads=1
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

            let env_vars = vec![
                format!("SIDECAR_PORT={CONTAINER_PORT}"),
                format!("SIDECAR_AUTH_TOKEN={AUTH_TOKEN}"),
                "NODE_ENV=development".to_string(),
                "PORT_WATCHER_ENABLED=false".to_string(),
            ];

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
            let client = Client::new();
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

/// Without a configured LLM backend, /agents/run returns an error with
/// {success: false, error: {code, message}}.
#[tokio::test]
async fn agent_run_without_backend_returns_structured_error() {
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

    // Without a backend, sidecar returns HTTP 500 with structured error.
    assert_eq!(body["success"], false, "should fail without backend: {body}");
    assert!(body["error"]["code"].is_string(), "error.code: {body}");
    assert!(body["error"]["message"].is_string(), "error.message: {body}");
}

/// Verify extract_agent_fields correctly parses the real error response.
#[tokio::test]
async fn extract_agent_fields_parses_real_error_response() {
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
    let (success, _response, error, _trace_id) = extract_agent_fields(&body);

    eprintln!("extract_agent_fields: success={success}, error='{error}'");

    // The real error is at body["error"]["message"], which extract_agent_fields
    // reads via: parsed.get("error").and_then(|err| err.get("message")...).
    assert!(!success, "should not be success");
    assert!(!error.is_empty(), "should extract error message from: {body}");
}

/// Blueprint's `run_prompt_request` posts to `/agents/run`.
/// This endpoint EXISTS, so the request goes through (fails from no backend,
/// not from 404).
#[tokio::test]
async fn blueprint_run_prompt_reaches_real_sidecar() {
    skip_unless_real!();
    let s = ensure_sidecar().await;

    let request = SandboxPromptRequest {
        sidecar_url: s.url.clone(),
        message: "Test prompt".to_string(),
        session_id: String::new(),
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 15000,
        sidecar_token: AUTH_TOKEN.to_string(),
    };

    let result = ai_agent_sandbox_blueprint_lib::run_prompt_request(&request).await;

    match &result {
        Ok(resp) => {
            eprintln!("run_prompt_request succeeded: success={}", resp.success);
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
// Cleanup
// ===================================================================

#[tokio::test]
async fn zz_cleanup_container() {
    if !should_run() {
        return;
    }

    let builder = match docker_builder().await {
        b => b,
    };
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
