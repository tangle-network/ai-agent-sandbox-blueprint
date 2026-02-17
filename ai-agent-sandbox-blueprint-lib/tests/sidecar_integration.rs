//! Integration tests for sidecar HTTP interactions, auth, response parsing,
//! batch operations, workflow scheduling, SSH commands, metrics tracking,
//! and utility functions.
//!
//! Uses `wiremock` to simulate the sidecar HTTP API. All mocks use the
//! actual sidecar response shapes: `/terminals/commands` for exec,
//! `/agents/run` for agent interactions.

use ai_agent_sandbox_blueprint_lib::extract_agent_fields;
use ai_agent_sandbox_blueprint_lib::http::{auth_headers, build_url, sidecar_post_json};
use ai_agent_sandbox_blueprint_lib::metrics::{OnChainMetrics, metrics};
use ai_agent_sandbox_blueprint_lib::util::{
    build_snapshot_command, merge_metadata, normalize_username, parse_json_object, shell_escape,
};
use ai_agent_sandbox_blueprint_lib::workflows::{
    WorkflowEntry, apply_workflow_execution, resolve_next_run,
};
use serde_json::json;
use std::sync::atomic::Ordering;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ─── HTTP Layer Tests ────────────────────────────────────────────────────────

mod http_tests {
    use super::*;

    #[test]
    fn build_url_basic() {
        let url = build_url("http://localhost:8080", "/terminals/commands").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/terminals/commands");
    }

    #[test]
    fn build_url_with_trailing_slash() {
        let url = build_url("http://localhost:8080/", "/terminals/commands").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/terminals/commands");
    }

    #[test]
    fn build_url_invalid_base_fails() {
        let result = build_url("not a url", "/terminals/commands");
        assert!(result.is_err());
    }

    #[test]
    fn auth_headers_sets_bearer_token() {
        let headers = auth_headers("my-secret-token").unwrap();
        assert_eq!(
            headers.get("authorization").unwrap().to_str().unwrap(),
            "Bearer my-secret-token"
        );
        assert_eq!(
            headers.get("content-type").unwrap().to_str().unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn sidecar_post_json_exec_success() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "hello world",
                    "stderr": "",
                    "duration": 50
                }
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({
                "command": "echo hello world"
            }),
        )
        .await
        .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["result"]["exitCode"], 0);
        assert_eq!(result["result"]["stdout"], "hello world");
        assert_eq!(result["result"]["stderr"], "");
    }

    #[tokio::test]
    async fn sidecar_post_json_agents_run_success() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "Task completed successfully",
                "traceId": "trace-abc-123",
                "durationMs": 1500,
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 50
                },
                "sessionId": "session-xyz"
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "test-token",
            json!({
                "identifier": "default",
                "message": "run the task"
            }),
        )
        .await
        .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["response"], "Task completed successfully");
        assert_eq!(result["traceId"], "trace-abc-123");
        assert_eq!(result["durationMs"], 1500);
        assert_eq!(result["usage"]["inputTokens"], 100);
        assert_eq!(result["usage"]["outputTokens"], 50);
        assert_eq!(result["sessionId"], "session-xyz");
    }

    #[tokio::test]
    async fn sidecar_post_json_http_error_propagates() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({"command": "fail"}),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("500"),
            "Error should mention status code: {err}"
        );
    }

    #[tokio::test]
    async fn sidecar_post_json_invalid_json_response() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({"command": "echo hi"}),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid sidecar response JSON"),
            "Error should mention invalid JSON: {err}"
        );
    }

    #[tokio::test]
    async fn sidecar_post_json_connection_refused() {
        let result = sidecar_post_json(
            "http://127.0.0.1:1",
            "/terminals/commands",
            "test-token",
            json!({"command": "echo hi"}),
        )
        .await;

        assert!(result.is_err());
    }
}

// ─── Response Parsing Tests ──────────────────────────────────────────────────

mod response_parsing_tests {
    use super::*;

    #[test]
    fn extract_agent_fields_success() {
        let parsed = json!({
            "success": true,
            "response": "Task done",
            "traceId": "trace-1",
            "error": null
        });
        let (success, response, error, trace_id) = extract_agent_fields(&parsed);
        assert!(success);
        assert_eq!(response, "Task done");
        assert_eq!(trace_id, "trace-1");
        assert_eq!(error, "");
    }

    #[test]
    fn extract_agent_fields_error_string() {
        let parsed = json!({
            "success": false,
            "error": "Something went wrong"
        });
        let (success, _response, error, _trace_id) = extract_agent_fields(&parsed);
        assert!(!success);
        assert_eq!(error, "Something went wrong");
    }

    #[test]
    fn extract_agent_fields_error_object_with_message() {
        let parsed = json!({
            "success": false,
            "error": {
                "message": "Rate limit exceeded",
                "code": 429
            }
        });
        let (success, _response, error, _trace_id) = extract_agent_fields(&parsed);
        assert!(!success);
        assert_eq!(error, "Rate limit exceeded");
    }

    #[test]
    fn extract_agent_fields_empty_response() {
        let parsed = json!({});
        let (success, response, error, trace_id) = extract_agent_fields(&parsed);
        assert!(!success);
        assert_eq!(response, "");
        assert_eq!(error, "");
        assert_eq!(trace_id, "");
    }
}

// ─── Agent Interaction Tests (with wiremock) ─────────────────────────────────

mod agent_interaction_tests {
    use super::*;

    #[tokio::test]
    async fn exec_command_with_env_and_cwd() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "/workspace\nNODE_ENV=production",
                    "stderr": "",
                    "duration": 30
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({
                "command": "pwd && env",
                "cwd": "/workspace",
                "env": {"NODE_ENV": "production"},
                "timeout": 5000
            }),
        )
        .await
        .unwrap();

        assert_eq!(result["result"]["exitCode"], 0);
        assert!(
            result["result"]["stdout"]
                .as_str()
                .unwrap()
                .contains("/workspace")
        );
    }

    #[tokio::test]
    async fn agent_run_with_session_and_model() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "Project scaffolded successfully",
                "sessionId": "session-persistent",
                "traceId": "trace-build",
                "durationMs": 5000,
                "usage": {"inputTokens": 500, "outputTokens": 200}
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "test-token",
            json!({
                "identifier": "default",
                "message": "Create a new React project with TypeScript",
                "sessionId": "session-persistent",
                "backend": {"model": "claude-sonnet-4-5-20250929"},
                "metadata": {"maxTurns": 10}
            }),
        )
        .await
        .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["sessionId"], "session-persistent");
        assert!(result["durationMs"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn agent_run_failure_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": false,
                "error": {"message": "Model rate limit exceeded", "code": 429},
                "traceId": "trace-err",
                "durationMs": 100,
                "usage": {"inputTokens": 10, "outputTokens": 0}
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "test-token",
            json!({
                "identifier": "default",
                "message": "Build a project"
            }),
        )
        .await
        .unwrap();

        let (success, _response, error, trace_id) = extract_agent_fields(&result);
        assert!(!success);
        assert_eq!(error, "Model rate limit exceeded");
        assert_eq!(trace_id, "trace-err");
    }

    #[tokio::test]
    async fn agent_multi_step_response() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "I've completed the following steps:\n1. Read the source files\n2. Made the changes\n3. Ran the tests",
                "traceId": "trace-multi-step",
                "durationMs": 15000,
                "usage": {"inputTokens": 2000, "outputTokens": 800},
                "sessionId": "session-build"
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "test-token",
            json!({
                "identifier": "default",
                "message": "Build a Node.js REST API",
                "metadata": {"maxTurns": 10, "maxSteps": 10}
            }),
        )
        .await
        .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["durationMs"].as_u64().unwrap() >= 15000);
        assert_eq!(result["usage"]["inputTokens"], 2000);
        assert_eq!(result["usage"]["outputTokens"], 800);
    }
}

// ─── Auth Tests ──────────────────────────────────────────────────────────────

mod auth_tests {
    use ai_agent_sandbox_blueprint_lib::auth::{
        generate_token, require_sidecar_token, token_from_request,
    };

    #[test]
    fn generate_token_produces_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_token_is_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn token_from_request_uses_override() {
        let token = token_from_request("my-custom-token");
        assert_eq!(token, "my-custom-token");
    }

    #[test]
    fn token_from_request_generates_when_empty() {
        let token = token_from_request("");
        assert_eq!(token.len(), 64);
    }

    #[test]
    fn token_from_request_generates_when_whitespace() {
        let token = token_from_request("   ");
        assert_eq!(token.len(), 64);
    }

    #[test]
    fn token_from_request_trims_override() {
        let token = token_from_request("  padded-token  ");
        assert_eq!(token, "padded-token");
    }

    #[test]
    fn require_sidecar_token_valid() {
        let result = require_sidecar_token("valid-token");
        assert_eq!(result.unwrap(), "valid-token");
    }

    #[test]
    fn require_sidecar_token_trims() {
        let result = require_sidecar_token("  spaced  ");
        assert_eq!(result.unwrap(), "spaced");
    }

    #[test]
    fn require_sidecar_token_empty_fails() {
        let result = require_sidecar_token("");
        assert!(result.is_err());
    }

    #[test]
    fn require_sidecar_token_whitespace_fails() {
        let result = require_sidecar_token("   ");
        assert!(result.is_err());
    }
}

// ─── JSON Parsing & Utility Tests ────────────────────────────────────────────

mod util_tests {
    use super::*;

    #[test]
    fn parse_json_object_valid() {
        let result = parse_json_object(r#"{"key": "value"}"#, "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap()["key"], "value");
    }

    #[test]
    fn parse_json_object_empty_string() {
        let result = parse_json_object("", "test").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_whitespace_only() {
        let result = parse_json_object("   ", "test").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_array_rejected() {
        let result = parse_json_object("[1, 2, 3]", "test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be a JSON object")
        );
    }

    #[test]
    fn parse_json_object_string_rejected() {
        let result = parse_json_object(r#""just a string""#, "test");
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_object_invalid_json_rejected() {
        let result = parse_json_object("{broken", "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid JSON"));
    }

    #[test]
    fn parse_json_object_nested() {
        let result =
            parse_json_object(r#"{"env": {"NODE_ENV": "prod"}, "debug": true}"#, "test").unwrap();
        let obj = result.unwrap();
        assert_eq!(obj["env"]["NODE_ENV"], "prod");
        assert_eq!(obj["debug"], true);
    }

    #[test]
    fn merge_metadata_empty_inputs() {
        let result = merge_metadata(None, "", "").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn merge_metadata_adds_image() {
        let result = merge_metadata(None, "agent-dev", "").unwrap();
        let obj = result.unwrap();
        assert_eq!(obj["image"], "agent-dev");
    }

    #[test]
    fn merge_metadata_adds_stack() {
        let result = merge_metadata(None, "", "node").unwrap();
        let obj = result.unwrap();
        assert_eq!(obj["stack"], "node");
    }

    #[test]
    fn merge_metadata_preserves_existing() {
        let existing = Some(json!({"custom": "value"}));
        let result = merge_metadata(existing, "img", "stk").unwrap();
        let obj = result.unwrap();
        assert_eq!(obj["custom"], "value");
        assert_eq!(obj["image"], "img");
        assert_eq!(obj["stack"], "stk");
    }

    #[test]
    fn merge_metadata_rejects_non_object() {
        let result = merge_metadata(Some(json!([1, 2])), "img", "");
        assert!(result.is_err());
    }

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn shell_escape_with_single_quotes() {
        let result = shell_escape("it's");
        assert_eq!(result, "'it'\"'\"'s'");
    }

    #[test]
    fn shell_escape_with_special_chars() {
        let result = shell_escape("echo $HOME && rm -rf /");
        assert!(result.starts_with('\''));
        assert!(result.ends_with('\''));
    }

    #[test]
    fn normalize_username_valid() {
        assert_eq!(normalize_username("root").unwrap(), "root");
        assert_eq!(normalize_username("user-1").unwrap(), "user-1");
        assert_eq!(normalize_username("user_2").unwrap(), "user_2");
        assert_eq!(normalize_username("user.name").unwrap(), "user.name");
    }

    #[test]
    fn normalize_username_trims() {
        assert_eq!(normalize_username("  root  ").unwrap(), "root");
    }

    #[test]
    fn normalize_username_empty_defaults_to_root() {
        assert_eq!(normalize_username("").unwrap(), "root");
        assert_eq!(normalize_username("   ").unwrap(), "root");
    }

    #[test]
    fn normalize_username_rejects_special_chars() {
        assert!(normalize_username("user;rm -rf /").is_err());
        assert!(normalize_username("user$(cmd)").is_err());
        assert!(normalize_username("user`cmd`").is_err());
        assert!(normalize_username("user/path").is_err());
        assert!(normalize_username("user name").is_err());
    }

    #[test]
    fn build_snapshot_command_workspace_only() {
        let cmd = build_snapshot_command("https://example.com/upload", true, false).unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(!cmd.contains("/var/lib/sidecar"));
        assert!(cmd.contains("tar -czf"));
        assert!(cmd.contains("curl"));
    }

    #[test]
    fn build_snapshot_command_state_only() {
        let cmd = build_snapshot_command("https://example.com/upload", false, true).unwrap();
        assert!(!cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn build_snapshot_command_both() {
        let cmd = build_snapshot_command("https://example.com/upload", true, true).unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn build_snapshot_command_neither_fails() {
        let result = build_snapshot_command("https://example.com/upload", false, false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must include workspace or state")
        );
    }

    #[test]
    fn build_snapshot_command_escapes_destination() {
        let cmd = build_snapshot_command("https://evil.com/'; rm -rf /; '", true, false).unwrap();
        assert!(
            cmd.contains("'\"'\"'"),
            "Single quotes should be escaped with quote-break pattern"
        );
        assert!(cmd.contains("curl -fsSL -X PUT --upload-file \"$tmp\" '"));
    }
}

// ─── Workflow Scheduling Tests ───────────────────────────────────────────────

mod workflow_tests {
    use super::*;

    #[test]
    fn resolve_next_run_non_cron_returns_none() {
        let result = resolve_next_run("manual", "", None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn resolve_next_run_cron_returns_future_timestamp() {
        let now = ai_agent_sandbox_blueprint_lib::util::now_ts();
        let result = resolve_next_run("cron", "0 * * * * *", Some(now)).unwrap();
        assert!(result.is_some());
        let next = result.unwrap();
        assert!(next > now, "Next run should be after current time");
        assert!(next <= now + 61, "Next run should be within ~60 seconds");
    }

    #[test]
    fn resolve_next_run_invalid_cron_fails() {
        let result = resolve_next_run("cron", "not a cron expr", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid cron expression"));
    }

    #[test]
    fn resolve_next_run_cron_without_last_run() {
        let result = resolve_next_run("cron", "0 * * * * *", None).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn apply_workflow_execution_updates_timestamps() {
        let mut entry = WorkflowEntry {
            id: 1,
            name: "test".to_string(),
            workflow_json: "{}".to_string(),
            trigger_type: "cron".to_string(),
            trigger_config: "0 * * * * *".to_string(),
            sandbox_config_json: "{}".to_string(),
            active: true,
            next_run_at: None,
            last_run_at: None,
            owner: String::new(),
        };

        apply_workflow_execution(&mut entry, 1000, Some(2000));

        assert_eq!(entry.last_run_at, Some(1000));
        assert_eq!(entry.next_run_at, Some(2000));
    }

    #[test]
    fn apply_workflow_execution_clears_next_run() {
        let mut entry = WorkflowEntry {
            id: 1,
            name: "test".to_string(),
            workflow_json: "{}".to_string(),
            trigger_type: "manual".to_string(),
            trigger_config: "".to_string(),
            sandbox_config_json: "{}".to_string(),
            active: true,
            next_run_at: Some(999),
            last_run_at: None,
            owner: String::new(),
        };

        apply_workflow_execution(&mut entry, 1000, None);

        assert_eq!(entry.last_run_at, Some(1000));
        assert_eq!(entry.next_run_at, None);
    }
}

// ─── Metrics Tests ───────────────────────────────────────────────────────────

mod metrics_tests {
    use super::*;

    #[test]
    fn record_job_increments_counters() {
        let m = OnChainMetrics::new();
        m.record_job(100, 50, 25);
        m.record_job(200, 30, 15);

        assert_eq!(m.total_jobs.load(Ordering::Relaxed), 2);
        assert_eq!(m.total_duration_ms.load(Ordering::Relaxed), 300);
        assert_eq!(m.total_input_tokens.load(Ordering::Relaxed), 80);
        assert_eq!(m.total_output_tokens.load(Ordering::Relaxed), 40);
    }

    #[test]
    fn record_failure_increments() {
        let m = OnChainMetrics::new();
        m.record_failure();
        m.record_failure();
        assert_eq!(m.failed_jobs.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn sandbox_created_tracks_resources() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(2, 4096);
        m.record_sandbox_created(4, 8192);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 2);
        assert_eq!(m.peak_sandboxes.load(Ordering::Relaxed), 2);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 6);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 12288);
    }

    #[test]
    fn sandbox_deleted_releases_resources() {
        let m = OnChainMetrics::new();
        m.record_sandbox_created(4, 8192);
        m.record_sandbox_created(2, 4096);
        m.record_sandbox_deleted(4, 8192);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 1);
        assert_eq!(m.peak_sandboxes.load(Ordering::Relaxed), 2);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 2);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 4096);
    }

    #[test]
    fn sandbox_deleted_saturates_at_zero() {
        let m = OnChainMetrics::new();
        m.record_sandbox_deleted(100, 99999);

        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), 0);
        assert_eq!(m.allocated_memory_mb.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn session_guard_decrements_on_drop() {
        let m = metrics();

        let initial = m.active_sessions.load(Ordering::Relaxed);
        {
            let _guard = m.session_guard();
            assert_eq!(m.active_sessions.load(Ordering::Relaxed), initial + 1);
        }
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), initial);
    }

    #[test]
    fn session_guard_multiple_concurrent() {
        let m = metrics();
        let initial = m.active_sessions.load(Ordering::Relaxed);

        let _g1 = m.session_guard();
        let _g2 = m.session_guard();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), initial + 2);

        drop(_g1);
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), initial + 1);

        drop(_g2);
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), initial);
    }

    #[test]
    fn snapshot_contains_all_metrics() {
        let m = OnChainMetrics::new();
        m.record_job(500, 100, 50);
        m.record_sandbox_created(2, 4096);
        m.record_failure();

        let snapshot = m.snapshot();
        let keys: Vec<&str> = snapshot.iter().map(|(k, _)| k.as_str()).collect();

        assert!(keys.contains(&"total_jobs"));
        assert!(keys.contains(&"avg_duration_ms"));
        assert!(keys.contains(&"total_input_tokens"));
        assert!(keys.contains(&"total_output_tokens"));
        assert!(keys.contains(&"active_sandboxes"));
        assert!(keys.contains(&"peak_sandboxes"));
        assert!(keys.contains(&"active_sessions"));
        assert!(keys.contains(&"allocated_cpu_cores"));
        assert!(keys.contains(&"allocated_memory_mb"));
        assert!(keys.contains(&"failed_jobs"));
    }

    #[test]
    fn snapshot_avg_duration_correct() {
        let m = OnChainMetrics::new();
        m.record_job(100, 0, 0);
        m.record_job(300, 0, 0);

        let snapshot = m.snapshot();
        let avg = snapshot
            .iter()
            .find(|(k, _)| k == "avg_duration_ms")
            .map(|(_, v)| *v)
            .unwrap();
        assert_eq!(avg, 200);
    }

    #[test]
    fn snapshot_avg_duration_zero_jobs() {
        let m = OnChainMetrics::new();
        let snapshot = m.snapshot();
        let avg = snapshot
            .iter()
            .find(|(k, _)| k == "avg_duration_ms")
            .map(|(_, v)| *v)
            .unwrap();
        assert_eq!(avg, 0);
    }
}

// ─── Error Type Tests ────────────────────────────────────────────────────────

mod error_tests {
    use ai_agent_sandbox_blueprint_lib::SandboxError;

    #[test]
    fn error_display_auth() {
        let err = SandboxError::Auth("bad token".into());
        assert_eq!(err.to_string(), "auth error: bad token");
    }

    #[test]
    fn error_display_docker() {
        let err = SandboxError::Docker("container failed".into());
        assert_eq!(err.to_string(), "docker error: container failed");
    }

    #[test]
    fn error_display_http() {
        let err = SandboxError::Http("timeout".into());
        assert_eq!(err.to_string(), "http error: timeout");
    }

    #[test]
    fn error_display_validation() {
        let err = SandboxError::Validation("bad input".into());
        assert_eq!(err.to_string(), "validation error: bad input");
    }

    #[test]
    fn error_display_not_found() {
        let err = SandboxError::NotFound("sandbox-123".into());
        assert_eq!(err.to_string(), "not found: sandbox-123");
    }

    #[test]
    fn error_display_storage() {
        let err = SandboxError::Storage("disk full".into());
        assert_eq!(err.to_string(), "storage error: disk full");
    }

    #[test]
    fn error_into_string() {
        let err = SandboxError::Auth("denied".into());
        let s: String = err.into();
        assert_eq!(s, "auth error: denied");
    }
}

// ─── Full Sidecar Workflow Tests (wiremock) ──────────────────────────────────

mod sidecar_workflow_tests {
    use super::*;

    #[tokio::test]
    async fn full_project_build_workflow() {
        let server = MockServer::start().await;

        // Step 1: Agent scaffolds the project
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "I've created a new Express.js project with the following structure:\n- package.json\n- src/index.ts\n- tsconfig.json",
                "sessionId": "session-build-1",
                "traceId": "trace-scaffold",
                "durationMs": 8000,
                "usage": {"inputTokens": 500, "outputTokens": 300}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let scaffold_result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "build-token",
            json!({
                "identifier": "default",
                "message": "Create a new Express.js TypeScript project",
                "metadata": {"maxTurns": 5}
            }),
        )
        .await
        .unwrap();

        assert!(scaffold_result["success"].as_bool().unwrap());
        let session_id = scaffold_result["sessionId"].as_str().unwrap();
        assert!(!session_id.is_empty());

        // Step 2: Verify with terminal command
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "package.json\nsrc\ntsconfig.json\nnode_modules",
                    "stderr": "",
                    "duration": 20
                }
            })))
            .mount(&server)
            .await;

        let ls_result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "build-token",
            json!({"command": "ls /workspace"}),
        )
        .await
        .unwrap();

        assert_eq!(ls_result["result"]["exitCode"], 0);
        let stdout = ls_result["result"]["stdout"].as_str().unwrap();
        assert!(stdout.contains("package.json"));
        assert!(stdout.contains("src"));

        // Step 3: Continue conversation with session ID
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "I've added the /api/health endpoint returning {\"status\": \"ok\"}",
                "sessionId": "session-build-1",
                "traceId": "trace-endpoint",
                "durationMs": 5000,
                "usage": {"inputTokens": 800, "outputTokens": 200}
            })))
            .mount(&server)
            .await;

        let endpoint_result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "build-token",
            json!({
                "identifier": "default",
                "message": "Add a health check endpoint at /api/health",
                "sessionId": session_id
            }),
        )
        .await
        .unwrap();

        assert!(endpoint_result["success"].as_bool().unwrap());
        assert_eq!(endpoint_result["sessionId"], session_id);
    }

    #[tokio::test]
    async fn batch_exec_multiple_sidecars() {
        let server1 = MockServer::start().await;
        let server2 = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "sidecar-1-output",
                    "stderr": "",
                    "duration": 10
                }
            })))
            .mount(&server1)
            .await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "sidecar-2-output",
                    "stderr": "",
                    "duration": 10
                }
            })))
            .mount(&server2)
            .await;

        let r1 = sidecar_post_json(
            &server1.uri(),
            "/terminals/commands",
            "token-1",
            json!({"command": "echo sidecar-1"}),
        )
        .await
        .unwrap();

        let r2 = sidecar_post_json(
            &server2.uri(),
            "/terminals/commands",
            "token-2",
            json!({"command": "echo sidecar-2"}),
        )
        .await
        .unwrap();

        assert_eq!(r1["result"]["exitCode"], 0);
        assert_eq!(r2["result"]["exitCode"], 0);
        assert_eq!(r1["result"]["stdout"], "sidecar-1-output");
        assert_eq!(r2["result"]["stdout"], "sidecar-2-output");
    }

    #[tokio::test]
    async fn ssh_key_provisioning_via_terminal_commands() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "",
                    "stderr": "",
                    "duration": 50
                }
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({
                "command": "sh -c 'mkdir -p /root/.ssh && echo ssh-ed25519 AAAA >> /root/.ssh/authorized_keys'"
            }),
        )
        .await
        .unwrap();

        assert_eq!(result["result"]["exitCode"], 0);
    }

    #[tokio::test]
    async fn auth_failure_returns_401() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/agents/run",
            "bad-token",
            json!({"identifier": "default", "message": "hello"}),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("401"), "Should mention 401: {err}");
    }

    #[tokio::test]
    async fn sidecar_timeout_handling() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 124,
                    "stdout": "",
                    "stderr": "command timed out",
                    "duration": 1000
                }
            })))
            .mount(&server)
            .await;

        let result = sidecar_post_json(
            &server.uri(),
            "/terminals/commands",
            "test-token",
            json!({"command": "sleep 999", "timeout": 1000}),
        )
        .await
        .unwrap();

        assert_eq!(result["result"]["exitCode"], 124);
        assert_eq!(result["result"]["stderr"], "command timed out");
    }
}
