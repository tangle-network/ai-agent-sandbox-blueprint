//! Integration tests for the AI Agent Instance Blueprint.
//!
//! These tests exercise the core logic of each job handler via the public
//! `run_instance_*` functions — the same pattern used in the sandbox blueprint.
//! TangleArg/Caller extractors are thin wrappers; if the core logic works,
//! the Tangle handler works.
//!
//! All mocks use wiremock to simulate the sidecar HTTP API.
//! Response shapes match the real sidecar:
//!   - `/terminals/commands` → `{ success, result: { exitCode, stdout, stderr, duration } }`
//!   - `/agents/run` → `{ success, response, traceId, durationMs, usage, sessionId }`

use ai_agent_instance_blueprint_lib::*;
use serde_json::json;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static INIT: Once = Once::new();
static CTR: AtomicU64 = AtomicU64::new(0);

fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("instance-bp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        // SAFETY: tests run single-threaded during init; no concurrent env reads.
        unsafe {
            std::env::set_var("BLUEPRINT_STATE_DIR", dir.to_str().unwrap());
            std::env::set_var("SIDECAR_IMAGE", "nginx:alpine");
            std::env::set_var("SIDECAR_PULL_IMAGE", "true");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            std::env::set_var("REQUEST_TIMEOUT_SECS", "10");
        }
    });
}

fn uid() -> String {
    format!("inst-test-{}", CTR.fetch_add(1, Ordering::SeqCst))
}

/// Insert a fake sandbox record into the runtime store and return its ID.
fn insert_sandbox(url: &str, token: &str) -> String {
    init();
    let id = uid();
    runtime::sandboxes()
        .unwrap()
        .insert(
            id.clone(),
            SandboxRecord {
                id: id.clone(),
                container_id: format!("ctr-{id}"),
                sidecar_url: url.to_string(),
                sidecar_port: 0,
                ssh_port: None,
                token: token.to_string(),
                created_at: util::now_ts(),
                cpu_cores: 2,
                memory_mb: 4096,
                state: Default::default(),
                idle_timeout_seconds: 0,
                max_lifetime_seconds: 0,
                last_activity_at: util::now_ts(),
                stopped_at: None,
                snapshot_image_id: None,
                snapshot_s3_url: None,
                container_removed_at: None,
                image_removed_at: None,
                original_image: String::new(),
                base_env_json: String::new(),
                user_env_json: String::new(),
                snapshot_destination: None,
                tee_deployment_id: None,
                tee_metadata_json: None,
                name: String::new(),
                agent_identifier: String::new(),
                metadata_json: String::new(),
                disk_gb: 0,
                stack: String::new(),
                owner: String::new(),
            },
        )
        .unwrap();
    id
}

fn rm(id: &str) {
    let _ = runtime::sandboxes().unwrap().remove(id);
}

fn mock_exec_ok(stdout: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "result": {
            "exitCode": 0,
            "stdout": stdout,
            "stderr": "",
            "duration": 50
        }
    }))
}

fn mock_exec_fail(exit_code: u32, stderr: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "result": {
            "exitCode": exit_code,
            "stdout": "",
            "stderr": stderr,
            "duration": 10
        }
    }))
}

fn mock_agent_ok(label: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "response": label,
        "traceId": format!("t-{label}"), "durationMs": 100,
        "usage": {"inputTokens": 10, "outputTokens": 5},
        "sessionId": format!("s-{label}")
    }))
}

fn mock_agent_error(msg: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": false,
        "response": "",
        "error": {"message": msg},
        "traceId": "t-err", "durationMs": 10,
        "usage": {"inputTokens": 1, "outputTokens": 0}
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// EXEC TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod exec_tests {
    use super::*;

    #[tokio::test]
    async fn exec_basic_command() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(mock_exec_ok("hello world"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceExecRequest {
            command: "echo hello world".to_string(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };

        let resp = run_instance_exec(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert_eq!(resp.exit_code, 0);
        assert_eq!(resp.stdout, "hello world");
        assert!(resp.stderr.is_empty());
        rm(&id);
    }

    #[tokio::test]
    async fn exec_with_cwd_and_env() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_ok("ok"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceExecRequest {
            command: "ls".to_string(),
            cwd: "/tmp".to_string(),
            env_json: r#"{"FOO":"bar"}"#.to_string(),
            timeout_ms: 5000,
        };

        let resp = run_instance_exec(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert_eq!(resp.exit_code, 0);
        assert_eq!(resp.stdout, "ok");
        rm(&id);
    }

    #[tokio::test]
    async fn exec_nonzero_exit_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_fail(1, "command not found"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceExecRequest {
            command: "badcmd".to_string(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };

        let resp = run_instance_exec(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert_eq!(resp.exit_code, 1);
        assert_eq!(resp.stderr, "command not found");
        rm(&id);
    }

    #[tokio::test]
    async fn exec_sidecar_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceExecRequest {
            command: "echo hi".to_string(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };

        let result = run_instance_exec(&server.uri(), "tok", &id, &request).await;
        assert!(result.is_err());
        rm(&id);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROMPT TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod prompt_tests {
    use super::*;

    #[tokio::test]
    async fn prompt_basic() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(mock_agent_ok("Hello from AI"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstancePromptRequest {
            message: "Say hello".to_string(),
            session_id: String::new(),
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };

        let resp = run_instance_prompt(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(resp.success);
        assert_eq!(resp.response, "Hello from AI");
        assert_eq!(resp.trace_id, "t-Hello from AI");
        assert_eq!(resp.duration_ms, 100);
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
        rm(&id);
    }

    #[tokio::test]
    async fn prompt_with_session_and_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("response-1"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstancePromptRequest {
            message: "Continue".to_string(),
            session_id: "sess-123".to_string(),
            model: "gpt-4".to_string(),
            context_json: r#"{"key":"value"}"#.to_string(),
            timeout_ms: 30000,
        };

        let resp = run_instance_prompt(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(resp.success);
        assert_eq!(resp.response, "response-1");
        rm(&id);
    }

    #[tokio::test]
    async fn prompt_error_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_error("model overloaded"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstancePromptRequest {
            message: "Hello".to_string(),
            session_id: String::new(),
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };

        let resp = run_instance_prompt(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(!resp.success);
        assert_eq!(resp.error, "model overloaded");
        rm(&id);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TASK TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod task_tests {
    use super::*;

    #[tokio::test]
    async fn task_basic() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(mock_agent_ok("task result"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceTaskRequest {
            prompt: "Write a hello world script".to_string(),
            session_id: String::new(),
            max_turns: 0,
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };

        let resp = run_instance_task(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(resp.success);
        assert_eq!(resp.result, "task result");
        assert_eq!(resp.trace_id, "t-task result");
        assert_eq!(resp.session_id, "s-task result");
        rm(&id);
    }

    #[tokio::test]
    async fn task_with_max_turns() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("done"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceTaskRequest {
            prompt: "Complex task".to_string(),
            session_id: "s-multi".to_string(),
            max_turns: 5,
            model: "claude-sonnet".to_string(),
            context_json: r#"{"project":"test"}"#.to_string(),
            timeout_ms: 60000,
        };

        let resp = run_instance_task(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(resp.success);
        assert_eq!(resp.result, "done");
        rm(&id);
    }

    #[tokio::test]
    async fn task_error_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_error("timeout exceeded"))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceTaskRequest {
            prompt: "Long task".to_string(),
            session_id: String::new(),
            max_turns: 10,
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 1000,
        };

        let resp = run_instance_task(&server.uri(), "tok", &id, &request)
            .await
            .unwrap();

        assert!(!resp.success);
        assert_eq!(resp.error, "timeout exceeded");
        rm(&id);
    }

    #[tokio::test]
    async fn task_sidecar_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;

        let id = insert_sandbox(&server.uri(), "tok");
        let request = InstanceTaskRequest {
            prompt: "test".to_string(),
            session_id: String::new(),
            max_turns: 0,
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };

        let result = run_instance_task(&server.uri(), "tok", &id, &request).await;
        assert!(result.is_err());
        rm(&id);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SSH TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod ssh_tests {
    use super::*;

    #[tokio::test]
    async fn ssh_provision_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(mock_exec_ok("key added"))
            .mount(&server)
            .await;

        let result = provision_key(
            &server.uri(),
            "root",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test@host",
            "tok",
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ssh_revoke_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(mock_exec_ok("key removed"))
            .mount(&server)
            .await;

        let result = revoke_key(
            &server.uri(),
            "root",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test@host",
            "tok",
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ssh_provision_rejects_invalid_username() {
        let server = MockServer::start().await;
        // No mock needed — should fail before HTTP call.
        let result = provision_key(
            &server.uri(),
            "root; rm -rf /",
            "ssh-ed25519 AAAA key",
            "tok",
        )
        .await;

        assert!(result.is_err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPER FUNCTION TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod helper_tests {
    use super::*;

    #[test]
    fn build_exec_payload_minimal() {
        let payload = build_exec_payload("echo hi", "", "", 0);
        assert_eq!(payload["command"], "echo hi");
        assert!(!payload.contains_key("cwd"));
        assert!(!payload.contains_key("timeout"));
        assert!(!payload.contains_key("env"));
    }

    #[test]
    fn build_exec_payload_with_all_fields() {
        let payload = build_exec_payload("ls", "/tmp", r#"{"FOO":"bar"}"#, 5000);
        assert_eq!(payload["command"], "ls");
        assert_eq!(payload["cwd"], "/tmp");
        assert_eq!(payload["timeout"], 5000);
        assert!(payload.contains_key("env"));
    }

    #[test]
    fn extract_exec_fields_full() {
        let v = json!({
            "result": {
                "exitCode": 42,
                "stdout": "out",
                "stderr": "err"
            }
        });
        let (code, stdout, stderr) = extract_exec_fields(&v);
        assert_eq!(code, 42);
        assert_eq!(stdout, "out");
        assert_eq!(stderr, "err");
    }

    #[test]
    fn extract_exec_fields_missing() {
        let v = json!({});
        let (code, stdout, stderr) = extract_exec_fields(&v);
        assert_eq!(code, 0);
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn extract_agent_fields_success() {
        let v = json!({
            "success": true,
            "response": "hello",
            "traceId": "t1",
            "error": null
        });
        let (success, response, error, trace_id) = extract_agent_fields(&v);
        assert!(success);
        assert_eq!(response, "hello");
        assert!(error.is_empty());
        assert_eq!(trace_id, "t1");
    }

    #[test]
    fn extract_agent_fields_error() {
        let v = json!({
            "success": false,
            "response": "",
            "error": {"message": "rate limit"},
            "traceId": "t-err"
        });
        let (success, _response, error, trace_id) = extract_agent_fields(&v);
        assert!(!success);
        assert_eq!(error, "rate limit");
        assert_eq!(trace_id, "t-err");
    }

    #[test]
    fn extract_agent_fields_fallback_response() {
        let v = json!({
            "success": true,
            "data": {"finalText": "fallback text"},
            "traceId": ""
        });
        let (success, response, _error, _trace_id) = extract_agent_fields(&v);
        assert!(success);
        assert_eq!(response, "fallback text");
    }

    #[test]
    fn build_agent_payload_minimal() {
        let payload = build_agent_payload("hello", "", "", "", 0, None).unwrap();
        assert_eq!(payload["identifier"], "default");
        assert_eq!(payload["message"], "hello");
        assert!(!payload.contains_key("sessionId"));
        assert!(!payload.contains_key("backend"));
        assert!(!payload.contains_key("timeout"));
    }

    #[test]
    fn build_agent_payload_with_session_and_model() {
        let payload = build_agent_payload("hello", "sess-1", "gpt-4", "", 30000, None).unwrap();
        assert_eq!(payload["sessionId"], "sess-1");
        assert_eq!(payload["backend"]["model"], "gpt-4");
        assert_eq!(payload["timeout"], 30000);
    }

    #[test]
    fn build_agent_payload_with_context() {
        let payload = build_agent_payload("hello", "", "", r#"{"key":"val"}"#, 0, None).unwrap();
        assert_eq!(payload["metadata"]["key"], "val");
    }

    #[test]
    fn build_agent_payload_with_extra_metadata() {
        use serde_json::Map;
        let mut extra = Map::new();
        extra.insert("maxTurns".to_string(), json!(5));

        let payload = build_agent_payload("hello", "", "", "", 0, Some(extra)).unwrap();
        assert_eq!(payload["metadata"]["maxTurns"], 5);
    }

    #[test]
    fn parse_agent_response_full() {
        let v = json!({
            "success": true,
            "response": "done",
            "traceId": "t1",
            "durationMs": 500,
            "usage": {"inputTokens": 100, "outputTokens": 50},
            "sessionId": "sess-abc"
        });
        let resp = parse_agent_response(&v, "fallback");
        assert!(resp.success);
        assert_eq!(resp.response, "done");
        assert_eq!(resp.trace_id, "t1");
        assert_eq!(resp.duration_ms, 500);
        assert_eq!(resp.input_tokens, 100);
        assert_eq!(resp.output_tokens, 50);
        assert_eq!(resp.session_id, "sess-abc");
    }

    #[test]
    fn parse_agent_response_uses_fallback_session_id() {
        let v = json!({
            "success": true,
            "response": "ok",
            "traceId": "",
            "durationMs": 0,
            "usage": {}
        });
        let resp = parse_agent_response(&v, "my-fallback");
        assert_eq!(resp.session_id, "my-fallback");
    }

    #[test]
    fn parse_agent_response_nested_session_id() {
        let v = json!({
            "success": true,
            "data": {
                "metadata": {
                    "sessionId": "nested-sess"
                }
            }
        });
        let resp = parse_agent_response(&v, "fallback");
        assert_eq!(resp.session_id, "nested-sess");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INSTANCE STATE TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod instance_state_tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize instance-state tests — they all share a single `INSTANCE_STORE`
    /// singleton keyed by `"instance"`, so parallel execution causes races.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn instance_store_initializes() {
        init();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = instance_store();
        assert!(store.is_ok());
    }

    #[test]
    fn get_instance_sandbox_returns_none_when_empty() {
        init();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = clear_instance_sandbox();
        let result = get_instance_sandbox().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn require_instance_sandbox_errors_when_empty() {
        init();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = clear_instance_sandbox();
        let result = require_instance_sandbox();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not provisioned"));
    }

    #[test]
    fn set_and_get_instance_sandbox() {
        init();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let record = SandboxRecord {
            id: "test-instance".to_string(),
            container_id: "ctr-test".to_string(),
            sidecar_url: "http://localhost:9999".to_string(),
            sidecar_port: 9999,
            ssh_port: Some(2222),
            token: "secret".to_string(),
            created_at: util::now_ts(),
            cpu_cores: 4,
            memory_mb: 8192,
            state: Default::default(),
            idle_timeout_seconds: 3600,
            max_lifetime_seconds: 86400,
            last_activity_at: util::now_ts(),
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "test:latest".to_string(),
            base_env_json: String::new(),
            user_env_json: String::new(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
        };

        set_instance_sandbox(record).unwrap();
        let got = get_instance_sandbox().unwrap().unwrap();
        assert_eq!(got.id, "test-instance");
        assert_eq!(got.sidecar_url, "http://localhost:9999");
        assert_eq!(got.token, "secret");

        // Cleanup
        clear_instance_sandbox().unwrap();
    }

    #[test]
    fn clear_instance_sandbox_removes_record() {
        init();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let record = SandboxRecord {
            id: "to-clear".to_string(),
            container_id: "ctr-clear".to_string(),
            sidecar_url: "http://localhost:1111".to_string(),
            sidecar_port: 1111,
            ssh_port: None,
            token: "tok".to_string(),
            created_at: util::now_ts(),
            cpu_cores: 1,
            memory_mb: 512,
            state: Default::default(),
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: util::now_ts(),
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: String::new(),
            base_env_json: String::new(),
            user_env_json: String::new(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
        };

        set_instance_sandbox(record).unwrap();
        assert!(get_instance_sandbox().unwrap().is_some());

        clear_instance_sandbox().unwrap();
        assert!(get_instance_sandbox().unwrap().is_none());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// JOB CONSTANTS TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod job_constants_tests {
    use super::*;

    #[test]
    fn job_ids_are_sequential() {
        assert_eq!(JOB_PROVISION, 0);
        assert_eq!(JOB_EXEC, 1);
        assert_eq!(JOB_PROMPT, 2);
        assert_eq!(JOB_TASK, 3);
        assert_eq!(JOB_SSH_PROVISION, 4);
        assert_eq!(JOB_SSH_REVOKE, 5);
        assert_eq!(JOB_SNAPSHOT, 6);
        assert_eq!(JOB_DEPROVISION, 7);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ABI ENCODING TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod abi_tests {
    use super::*;
    use blueprint_sdk::alloy::sol_types::SolValue;

    #[test]
    fn exec_request_abi_roundtrip() {
        let request = InstanceExecRequest {
            command: "echo hello".to_string(),
            cwd: "/tmp".to_string(),
            env_json: r#"{"A":"1"}"#.to_string(),
            timeout_ms: 5000,
        };

        let encoded = request.abi_encode();
        let decoded = InstanceExecRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.command, "echo hello");
        assert_eq!(decoded.cwd, "/tmp");
        assert_eq!(decoded.timeout_ms, 5000);
    }

    #[test]
    fn exec_response_abi_roundtrip() {
        let response = InstanceExecResponse {
            exit_code: 42,
            stdout: "output".to_string(),
            stderr: "error".to_string(),
        };

        let encoded = response.abi_encode();
        let decoded = InstanceExecResponse::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.exit_code, 42);
        assert_eq!(decoded.stdout, "output");
        assert_eq!(decoded.stderr, "error");
    }

    #[test]
    fn prompt_request_abi_roundtrip() {
        let request = InstancePromptRequest {
            message: "Say hello".to_string(),
            session_id: "sess-1".to_string(),
            model: "gpt-4".to_string(),
            context_json: "{}".to_string(),
            timeout_ms: 30000,
        };

        let encoded = request.abi_encode();
        let decoded = InstancePromptRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.message, "Say hello");
        assert_eq!(decoded.session_id, "sess-1");
        assert_eq!(decoded.model, "gpt-4");
    }

    #[test]
    fn task_request_abi_roundtrip() {
        let request = InstanceTaskRequest {
            prompt: "Build a CLI tool".to_string(),
            session_id: String::new(),
            max_turns: 10,
            model: "claude-sonnet".to_string(),
            context_json: String::new(),
            timeout_ms: 120000,
        };

        let encoded = request.abi_encode();
        let decoded = InstanceTaskRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.prompt, "Build a CLI tool");
        assert_eq!(decoded.max_turns, 10);
        assert_eq!(decoded.model, "claude-sonnet");
    }

    #[test]
    fn provision_request_abi_roundtrip() {
        let request = ProvisionRequest {
            name: "my-sandbox".to_string(),
            image: "ubuntu:22.04".to_string(),
            stack: "python".to_string(),
            agent_identifier: "agent-1".to_string(),
            env_json: "{}".to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: true,
            ssh_public_key: "ssh-ed25519 AAAA".to_string(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 86400,
            idle_timeout_seconds: 3600,
            cpu_cores: 2,
            memory_mb: 4096,
            disk_gb: 10,
            sidecar_token: "token".to_string(),
            tee_required: true,
            tee_type: 2, // Nitro
        };

        let encoded = request.abi_encode();
        let decoded = ProvisionRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.name, "my-sandbox");
        assert!(decoded.ssh_enabled);
        assert!(decoded.tee_required);
        assert_eq!(decoded.tee_type, 2);
    }

    #[test]
    fn provision_output_abi_roundtrip() {
        let output = ProvisionOutput {
            sandbox_id: "sb-123".to_string(),
            sidecar_url: "http://localhost:8080".to_string(),
            ssh_port: 2222,
            tee_attestation_json: r#"{"tee_type":"nitro"}"#.to_string(),
        };

        let encoded = output.abi_encode();
        let decoded = ProvisionOutput::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.sandbox_id, "sb-123");
        assert_eq!(decoded.sidecar_url, "http://localhost:8080");
        assert_eq!(decoded.ssh_port, 2222);
    }

    #[test]
    fn ssh_provision_request_abi_roundtrip() {
        let request = InstanceSshProvisionRequest {
            username: "root".to_string(),
            public_key: "ssh-ed25519 AAAA test@host".to_string(),
        };

        let encoded = request.abi_encode();
        let decoded = InstanceSshProvisionRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.username, "root");
        assert_eq!(decoded.public_key, "ssh-ed25519 AAAA test@host");
    }

    #[test]
    fn snapshot_request_abi_roundtrip() {
        let request = InstanceSnapshotRequest {
            destination: "s3://bucket/snapshot".to_string(),
            include_workspace: true,
            include_state: false,
        };

        let encoded = request.abi_encode();
        let decoded = InstanceSnapshotRequest::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.destination, "s3://bucket/snapshot");
        assert!(decoded.include_workspace);
        assert!(!decoded.include_state);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROVISION PARAMS CONVERSION TESTS
// ═══════════════════════════════════════════════════════════════════════════

mod conversion_tests {
    use super::*;

    #[test]
    fn provision_request_to_create_sandbox_params() {
        let request = ProvisionRequest {
            name: "test-sb".to_string(),
            image: "ubuntu:22.04".to_string(),
            stack: "python".to_string(),
            agent_identifier: "agent-1".to_string(),
            env_json: r#"{"K":"V"}"#.to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: true,
            ssh_public_key: "ssh-rsa AAAA".to_string(),
            web_terminal_enabled: true,
            max_lifetime_seconds: 86400,
            idle_timeout_seconds: 1800,
            cpu_cores: 4,
            memory_mb: 8192,
            disk_gb: 50,
            sidecar_token: "my-token".to_string(),
            tee_required: true,
            tee_type: 1, // Sgx
        };

        let params = CreateSandboxParams::from(&request);
        assert_eq!(params.name, "test-sb");
        assert_eq!(params.image, "ubuntu:22.04");
        assert_eq!(params.stack, "python");
        assert_eq!(params.agent_identifier, "agent-1");
        assert!(params.ssh_enabled);
        assert_eq!(params.cpu_cores, 4);
        assert_eq!(params.memory_mb, 8192);
        let tee = params.tee_config.unwrap();
        assert!(tee.required);
        assert!(matches!(tee.tee_type, TeeType::Sgx));
    }

    #[test]
    fn provision_request_no_tee() {
        let request = ProvisionRequest {
            name: "no-tee".to_string(),
            image: "alpine".to_string(),
            stack: String::new(),
            agent_identifier: String::new(),
            env_json: String::new(),
            metadata_json: String::new(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 0,
            idle_timeout_seconds: 0,
            cpu_cores: 1,
            memory_mb: 512,
            disk_gb: 5,
            sidecar_token: String::new(),
            tee_required: false,
            tee_type: 0,
        };

        let params = CreateSandboxParams::from(&request);
        assert!(params.tee_config.is_none());
    }

    #[test]
    fn provision_request_tee_types() {
        for (tee_type_id, expected) in [
            (0u8, TeeType::None),
            (1, TeeType::Sgx),
            (2, TeeType::Nitro),
            (3, TeeType::Sev),
            (99, TeeType::None), // unknown falls back to None
        ] {
            let request = ProvisionRequest {
                name: String::new(),
                image: String::new(),
                stack: String::new(),
                agent_identifier: String::new(),
                env_json: String::new(),
                metadata_json: String::new(),
                ssh_enabled: false,
                ssh_public_key: String::new(),
                web_terminal_enabled: false,
                max_lifetime_seconds: 0,
                idle_timeout_seconds: 0,
                cpu_cores: 0,
                memory_mb: 0,
                disk_gb: 0,
                sidecar_token: String::new(),
                tee_required: true,
                tee_type: tee_type_id,
            };

            let params = CreateSandboxParams::from(&request);
            let tee = params.tee_config.unwrap();
            assert_eq!(
                std::mem::discriminant(&tee.tee_type),
                std::mem::discriminant(&expected),
                "tee_type {tee_type_id} should map to {expected:?}"
            );
        }
    }
}
