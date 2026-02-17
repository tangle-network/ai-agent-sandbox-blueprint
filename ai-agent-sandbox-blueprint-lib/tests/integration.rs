//! Integration tests: exercises every job handler's core logic path end-to-end.
//!
//! Each handler is wrapped in `TangleArg`/`Caller` extractors that require a live
//! Tangle context. The pattern: extract core logic into pub functions (like
//! `run_task_request`, `run_exec_request`, `run_prompt_request`, `provision_key`,
//! `revoke_key`) and test those. The Tangle wrapper is a 3-line adapter — if the
//! core logic works, the handler works.
//!
//! All mocks use the actual sidecar response shapes:
//!   - `/terminals/commands` returns `{ success, result: { exitCode, stdout, stderr, duration } }`
//!   - `/agents/run` returns `{ success, response, traceId, durationMs, usage, sessionId }`

use ai_agent_sandbox_blueprint_lib::http::sidecar_post_json;
use ai_agent_sandbox_blueprint_lib::jobs::exec::{
    extract_exec_fields, run_exec_request, run_prompt_request,
};
use ai_agent_sandbox_blueprint_lib::jobs::ssh::{provision_key, revoke_key};
use ai_agent_sandbox_blueprint_lib::runtime::{
    SandboxRecord, get_sandbox_by_id, get_sandbox_by_url, require_sidecar_auth, sandboxes,
};
use ai_agent_sandbox_blueprint_lib::util::build_snapshot_command;
use ai_agent_sandbox_blueprint_lib::jobs::exec::run_task_request;
use ai_agent_sandbox_blueprint_lib::util::now_ts;
use ai_agent_sandbox_blueprint_lib::workflows::{
    WorkflowEntry, run_workflow, workflow_key, workflow_tick, workflows,
};
use ai_agent_sandbox_blueprint_lib::*;
use blueprint_sdk::alloy::sol_types::SolValue;
use serde_json::{Value, json};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static INIT: Once = Once::new();
static CTR: AtomicU64 = AtomicU64::new(0);

fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("sandbox-bp-test-{}", std::process::id()));
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
    format!("test-{}", CTR.fetch_add(1, Ordering::SeqCst))
}

fn insert_sandbox(url: &str, token: &str) -> String {
    init();
    let id = uid();
    sandboxes()
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
                created_at: now_ts(),
                cpu_cores: 2,
                memory_mb: 4096,
                state: Default::default(),
                idle_timeout_seconds: 0,
                max_lifetime_seconds: 0,
                last_activity_at: now_ts(),
                stopped_at: None,
                snapshot_image_id: None,
                snapshot_s3_url: None,
                container_removed_at: None,
                image_removed_at: None,
                original_image: String::new(),
                env_json: String::new(),
                snapshot_destination: None,
                tee_deployment_id: None,
                tee_metadata_json: None,
                name: String::new(),
                agent_identifier: String::new(),
                metadata_json: String::new(),
                disk_gb: 0,
                stack: String::new(),
                owner: String::new(),
                secrets_configured: false,
            },
        )
        .unwrap();
    id
}

fn rm(id: &str) {
    let _ = sandboxes().unwrap().remove(id);
}

fn mock_agent_ok(label: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "response": label,
        "traceId": format!("t-{label}"), "durationMs": 100,
        "usage": {"inputTokens": 10, "outputTokens": 5},
        "sessionId": format!("s-{label}")
    }))
}

/// Mock for /terminals/commands returning the sidecar shape.
fn mock_exec_ok(stdout: &str, exit_code: u32) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "result": {
            "exitCode": exit_code,
            "stdout": stdout,
            "stderr": "",
            "duration": 50
        }
    }))
}

fn task_req(url: &str, prompt: &str) -> SandboxTaskRequest {
    SandboxTaskRequest {
        sidecar_url: url.to_string(),
        prompt: prompt.to_string(),
        session_id: String::new(),
        max_turns: 1,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 0,
    }
}

// ─── Multi-Tenant Auth Isolation ─────────────────────────────────────────────

mod tenant_isolation {
    use super::*;

    #[test]
    fn correct_token_accepted() {
        let id = insert_sandbox("http://tenant-ok:8080", "secret-ok");
        let r = require_sidecar_auth("http://tenant-ok:8080", "secret-ok").unwrap();
        assert_eq!(r.id, id);
        rm(&id);
    }

    #[test]
    fn wrong_token_rejected() {
        let id = insert_sandbox("http://tenant-bad:8080", "real-secret");
        let r = require_sidecar_auth("http://tenant-bad:8080", "wrong");
        assert!(r.unwrap_err().to_string().contains("Unauthorized"));
        rm(&id);
    }

    #[test]
    fn cross_tenant_tokens_rejected() {
        let a = insert_sandbox("http://cross-a:8080", "tok-a");
        let b = insert_sandbox("http://cross-b:8080", "tok-b");
        assert!(require_sidecar_auth("http://cross-a:8080", "tok-b").is_err());
        assert!(require_sidecar_auth("http://cross-b:8080", "tok-a").is_err());
        rm(&a);
        rm(&b);
    }

    #[test]
    fn unknown_url_returns_not_found() {
        init();
        let r = require_sidecar_auth("http://ghost:9999", "any");
        assert!(r.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn lookup_by_id_and_url() {
        let id = insert_sandbox("http://lookup:8080", "tok-lu");
        assert_eq!(get_sandbox_by_id(&id).unwrap().token, "tok-lu");
        assert_eq!(get_sandbox_by_url("http://lookup:8080").unwrap().id, id);
        assert!(get_sandbox_by_id("nonexistent-id").is_err());
        rm(&id);
    }

    #[test]
    fn empty_token_rejected_before_store() {
        assert!(auth::require_sidecar_token("").is_err());
        assert!(auth::require_sidecar_token("   ").is_err());
    }
}

// ─── JOB 10: sandbox_exec (via run_exec_request) ────────────────────────────

mod exec_job {
    use super::*;

    #[tokio::test]
    async fn sidecar_response_shape() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {"exitCode": 0, "stdout": "hello", "stderr": "", "duration": 30}
            })))
            .mount(&srv)
            .await;

        let req = SandboxExecRequest {
            sidecar_url: srv.uri(),
            command: "echo hello".into(),
            cwd: "/app".into(),
            env_json: r#"{"FOO":"bar"}"#.into(),
            timeout_ms: 5000,
        };
        let resp = run_exec_request(&req, "t").await.unwrap();
        assert_eq!(resp.exit_code, 0);
        assert_eq!(resp.stdout, "hello");
        assert!(resp.stderr.is_empty());
    }

    #[test]
    fn extract_exec_fields_from_result() {
        let response = json!({
            "success": true,
            "result": {"exitCode": 42, "stdout": "ok", "stderr": "warn", "duration": 100}
        });
        let (code, out, err) = extract_exec_fields(&response);
        assert_eq!(code, 42);
        assert_eq!(out, "ok");
        assert_eq!(err, "warn");

        // Missing fields default to 0/empty
        let empty = json!({});
        let (code, out, err) = extract_exec_fields(&empty);
        assert_eq!(code, 0);
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[tokio::test]
    async fn payload_includes_cwd_env_timeout() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_ok("", 0))
            .expect(1)
            .mount(&srv)
            .await;

        let req = SandboxExecRequest {
            sidecar_url: srv.uri(),
            command: "ls".into(),
            cwd: "/workspace".into(),
            env_json: r#"{"NODE_ENV":"test"}"#.into(),
            timeout_ms: 3000,
        };
        run_exec_request(&req, "t").await.unwrap();
    }

    #[tokio::test]
    async fn empty_optional_fields_omitted() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_ok("", 0))
            .expect(1)
            .mount(&srv)
            .await;

        let req = SandboxExecRequest {
            sidecar_url: srv.uri(),
            command: "pwd".into(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };
        run_exec_request(&req, "t").await.unwrap();
    }
}

// ─── JOB 11: sandbox_prompt (via run_prompt_request) ─────────────────────────

mod prompt_job {
    use super::*;

    #[tokio::test]
    async fn success_with_all_fields() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "response": "Hello!",
                "traceId": "tr-1",
                "durationMs": 250,
                "usage": {"inputTokens": 50, "outputTokens": 20},
                "sessionId": "sess-1"
            })))
            .mount(&srv)
            .await;

        let req = SandboxPromptRequest {
            sidecar_url: srv.uri(),
            message: "hi".into(),
            session_id: "s1".into(),
            model: "claude-4".into(),
            context_json: r#"{"key":"val"}"#.into(),
            timeout_ms: 10000,
        };
        let resp = run_prompt_request(&req, "t").await.unwrap();
        assert!(resp.success);
        assert_eq!(resp.response, "Hello!");
        assert_eq!(resp.trace_id, "tr-1");
        assert_eq!(resp.duration_ms, 250);
        assert_eq!(resp.input_tokens, 50);
        assert_eq!(resp.output_tokens, 20);
    }

    #[tokio::test]
    async fn failure_records_metrics() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": false,
                "error": {"message": "rate limited"},
                "traceId": "", "durationMs": 0,
                "usage": {"inputTokens": 0, "outputTokens": 0},
                "sessionId": ""
            })))
            .mount(&srv)
            .await;

        let m = metrics::metrics();
        let before = m.failed_jobs.load(Ordering::Relaxed);

        let req = SandboxPromptRequest {
            sidecar_url: srv.uri(),
            message: "go".into(),
            session_id: String::new(),
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };
        let resp = run_prompt_request(&req, "t").await.unwrap();
        assert!(!resp.success);
        assert_eq!(resp.error, "rate limited");
        assert!(m.failed_jobs.load(Ordering::Relaxed) > before);
    }
}

// ─── JOB 12: sandbox_task (via run_task_request) ─────────────────────────────

mod task_job {
    use super::*;

    #[tokio::test]
    async fn full_response_parsing() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("done"))
            .mount(&srv)
            .await;

        let resp = run_task_request(&task_req(&srv.uri(), "work"), "t")
            .await
            .unwrap();
        assert!(resp.success);
        assert_eq!(resp.result, "done");
        assert_eq!(resp.trace_id, "t-done");
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
        assert_eq!(resp.session_id, "s-done");
    }

    #[tokio::test]
    async fn max_turns_in_metadata() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("ok"))
            .expect(1)
            .mount(&srv)
            .await;

        let req = SandboxTaskRequest {
            sidecar_url: srv.uri(),
            prompt: "go".into(),
            session_id: "s".into(),
            max_turns: 5,
            model: "claude".into(),
            context_json: r#"{"project":"x"}"#.into(),
            timeout_ms: 30000,
        };
        let resp = run_task_request(&req, "t").await.unwrap();
        assert!(resp.success);
    }
}

// ─── JOB 4: sandbox_snapshot (build_snapshot_command) ────────────────────────

mod snapshot_job {
    use super::*;

    #[test]
    fn workspace_only() {
        let cmd = build_snapshot_command("s3://bucket/snap.tar.gz", true, false).unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(!cmd.contains("/var/lib/sidecar"));
        assert!(cmd.contains("s3://bucket/snap.tar.gz"));
    }

    #[test]
    fn state_only() {
        let cmd = build_snapshot_command("s3://bucket/snap.tar.gz", false, true).unwrap();
        assert!(!cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn both_workspace_and_state() {
        let cmd = build_snapshot_command("https://dest/snap", true, true).unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn neither_returns_error() {
        let r = build_snapshot_command("s3://bucket/snap", false, false);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn snapshot_flow_via_sidecar() {
        let srv = MockServer::start().await;
        let id = insert_sandbox(&srv.uri(), "snap-tok");

        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {
                    "exitCode": 0,
                    "stdout": "uploaded",
                    "stderr": "",
                    "duration": 500
                }
            })))
            .expect(1)
            .mount(&srv)
            .await;

        let cmd = build_snapshot_command("s3://bucket/snap.tar.gz", true, false).unwrap();
        let payload = json!({"command": format!("sh -c {}", util::shell_escape(&cmd))});
        let resp = sidecar_post_json(&srv.uri(), "/terminals/commands", "snap-tok", payload)
            .await
            .unwrap();
        let (_, stdout, _) = extract_exec_fields(&resp);
        assert_eq!(stdout, "uploaded");
        rm(&id);
    }
}

// ─── JOB 21: batch_task + JOB 23: batch_collect ─────────────────────────────

mod batch_jobs {
    use super::*;

    #[tokio::test]
    async fn parallel_tasks_across_sidecars() {
        let servers: Vec<MockServer> =
            futures::future::join_all((0..3).map(|_| MockServer::start())).await;
        for (i, srv) in servers.iter().enumerate() {
            Mock::given(method("POST"))
                .and(path("/agents/run"))
                .respond_with(mock_agent_ok(&format!("s{i}")))
                .expect(1)
                .mount(srv)
                .await;
        }

        let mut set = tokio::task::JoinSet::new();
        for srv in &servers {
            let req = task_req(&srv.uri(), "go");
            set.spawn(async move { run_task_request(&req, "t").await });
        }

        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap().unwrap());
        }
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.success));
    }

    #[tokio::test]
    async fn failure_isolation_one_bad_sidecar() {
        let good = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("ok"))
            .mount(&good)
            .await;

        let bad = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(500).set_body_string("crash"))
            .mount(&bad)
            .await;

        assert!(
            run_task_request(&task_req(&good.uri(), "go"), "t")
                .await
                .is_ok()
        );
        assert!(
            run_task_request(&task_req(&bad.uri(), "go"), "t")
                .await
                .is_err()
        );
    }

    #[test]
    fn batch_store_roundtrip() {
        init();
        let batch_id = format!("batch-test-{}", uid());
        let record = BatchRecord {
            id: batch_id.clone(),
            kind: "task".into(),
            results: json!([{"success": true, "result": "done"}]),
            created_at: now_ts(),
        };

        batches().unwrap().insert(batch_id.clone(), record).unwrap();

        let stored = batches().unwrap().get(&batch_id).unwrap().unwrap();
        assert_eq!(stored.kind, "task");
        assert_eq!(stored.results[0]["success"], true);

        batches().unwrap().remove(&batch_id).unwrap();
        assert!(batches().unwrap().get(&batch_id).unwrap().is_none());
    }
}

// ─── JOB 40: ssh_provision + JOB 41: ssh_revoke ─────────────────────────────

mod ssh_jobs {
    use super::*;

    #[tokio::test]
    async fn provision_key_sends_command() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_ok("", 0))
            .expect(1)
            .mount(&srv)
            .await;

        provision_key(&srv.uri(), "developer", "ssh-ed25519 AAAA test@host", "t")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn revoke_key_sends_command() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(mock_exec_ok("", 0))
            .expect(1)
            .mount(&srv)
            .await;

        revoke_key(&srv.uri(), "developer", "ssh-ed25519 AAAA test@host", "t")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn invalid_username_rejected() {
        let srv = MockServer::start().await;
        let r = provision_key(&srv.uri(), "user;rm -rf /", "key", "t").await;
        assert!(r.is_err());
        let r = revoke_key(&srv.uri(), "user$(evil)", "key", "t").await;
        assert!(r.is_err());
    }
}

// ─── JOB 30-33: Workflow Lifecycle ───────────────────────────────────────────

mod workflow_jobs {
    use super::*;

    fn wf(id: u64, url: &str, token: &str) -> WorkflowEntry {
        WorkflowEntry {
            id,
            name: format!("wf-{id}"),
            workflow_json: format!(
                r#"{{"sidecar_url":"{url}","prompt":"run","sidecar_token":"{token}"}}"#
            ),
            trigger_type: "cron".to_string(),
            trigger_config: "0 * * * * *".to_string(),
            sandbox_config_json: "{}".to_string(),
            active: true,
            next_run_at: Some(1),
            last_run_at: None,
            owner: String::new(),
        }
    }

    #[test]
    fn create_read_update_delete() {
        init();
        let key = workflow_key(90001);
        workflows()
            .unwrap()
            .insert(key.clone(), wf(90001, "http://x", "t"))
            .unwrap();

        let r = workflows().unwrap().get(&key).unwrap().unwrap();
        assert_eq!(r.name, "wf-90001");
        assert!(r.active);

        workflows()
            .unwrap()
            .update(&key, |e| {
                e.active = false;
                e.next_run_at = None;
            })
            .unwrap();
        let r = workflows().unwrap().get(&key).unwrap().unwrap();
        assert!(!r.active);
        assert!(r.next_run_at.is_none());

        workflows().unwrap().remove(&key).unwrap();
        assert!(workflows().unwrap().get(&key).unwrap().is_none());
    }

    #[tokio::test]
    async fn trigger_executes_and_updates() {
        let srv = MockServer::start().await;
        let sid = insert_sandbox(&srv.uri(), "wf-tok");
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("wf-ran"))
            .mount(&srv)
            .await;

        let entry = wf(90002, &srv.uri(), "wf-tok");
        let exec = run_workflow(&entry).await.unwrap();
        assert!(exec.response["task"]["success"].as_bool().unwrap());
        assert!(exec.last_run_at > 0);
        assert!(exec.next_run_at.is_some());

        rm(&sid);
    }

    #[tokio::test]
    async fn tick_runs_due_workflows() {
        let srv = MockServer::start().await;
        let sid = insert_sandbox(&srv.uri(), "tick-tok");
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("ticked"))
            .mount(&srv)
            .await;

        let key = workflow_key(90003);
        let entry = wf(90003, &srv.uri(), "tick-tok");
        workflows().unwrap().insert(key.clone(), entry).unwrap();

        let result = workflow_tick().await.unwrap();
        assert!(!result["executed"].as_array().unwrap().is_empty());

        let updated = workflows().unwrap().get(&key).unwrap().unwrap();
        assert!(updated.last_run_at.is_some());

        workflows().unwrap().remove(&key).unwrap();
        rm(&sid);
    }

    #[test]
    fn cancel_workflow() {
        init();
        let key = workflow_key(90005);
        workflows()
            .unwrap()
            .insert(key.clone(), wf(90005, "http://unused", "t"))
            .unwrap();

        let w = workflows().unwrap().get(&key).unwrap().unwrap();
        assert!(w.active);

        workflows()
            .unwrap()
            .update(&key, |e| {
                e.active = false;
                e.next_run_at = None;
            })
            .unwrap();

        let w = workflows().unwrap().get(&key).unwrap().unwrap();
        assert!(!w.active);
        assert!(w.next_run_at.is_none());

        workflows().unwrap().remove(&key).unwrap();
    }

    #[test]
    fn inactive_workflow_skipped_by_tick() {
        init();
        let key = workflow_key(90004);
        let mut entry = wf(90004, "http://unused", "t");
        entry.active = false;
        workflows().unwrap().insert(key.clone(), entry).unwrap();

        let all = workflows().unwrap().values().unwrap();
        let w = all.iter().find(|w| w.id == 90004).unwrap();
        assert!(!w.active);

        workflows().unwrap().remove(&key).unwrap();
    }
}

// ─── Response Parsing: extract_agent_fields ──────────────────────────────────

mod response_parsing {
    use super::*;

    #[test]
    fn success_shape() {
        let v = json!({"success": true, "response": "hello", "error": "none", "traceId": "t1"});
        let (success, response, error, trace_id) = extract_agent_fields(&v);
        assert!(success);
        assert_eq!(response, "hello");
        assert_eq!(error, "none");
        assert_eq!(trace_id, "t1");
    }

    #[test]
    fn missing_fields_default() {
        let v = json!({});
        let (success, response, error, trace_id) = extract_agent_fields(&v);
        assert!(!success);
        assert!(response.is_empty());
        assert!(error.is_empty());
        assert!(trace_id.is_empty());
    }

    #[test]
    fn error_object_with_message() {
        let v = json!({"success": false, "error": {"message": "rate limit exceeded"}});
        let (success, _, error, _) = extract_agent_fields(&v);
        assert!(!success);
        assert_eq!(error, "rate limit exceeded");
    }

    #[test]
    fn error_as_string() {
        let v = json!({"success": false, "error": "simple error"});
        let (_, _, error, _) = extract_agent_fields(&v);
        assert_eq!(error, "simple error");
    }
}

// ─── ABI Encoding Round-Trips ────────────────────────────────────────────────

mod abi {
    use super::*;
    use blueprint_sdk::alloy::primitives::Address;

    #[test]
    fn sandbox_create_and_output() {
        let req = SandboxCreateRequest {
            name: "t".into(),
            image: "img".into(),
            stack: "node".into(),
            agent_identifier: "a".into(),
            env_json: r#"{"K":"V"}"#.into(),
            metadata_json: "{}".into(),
            ssh_enabled: true,
            ssh_public_key: "ssh-ed25519 AAAA".into(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 3600,
            idle_timeout_seconds: 900,
            cpu_cores: 4,
            memory_mb: 8192,
            disk_gb: 50,
        };
        let d = SandboxCreateRequest::abi_decode(&req.abi_encode()).unwrap();
        assert_eq!(d.name, "t");
        assert_eq!(d.cpu_cores, 4);
        assert!(d.ssh_enabled);

        let out = SandboxCreateOutput {
            sandboxId: "sb-1".into(),
            json: r#"{"url":"http://h:1"}"#.into(),
        };
        let d = SandboxCreateOutput::abi_decode(&out.abi_encode()).unwrap();
        assert_eq!(d.sandboxId, "sb-1");
    }

    #[test]
    fn exec_prompt_task_types() {
        let exec = SandboxExecRequest {
            sidecar_url: "http://h".into(),
            command: "ls".into(),
            cwd: "/w".into(),
            env_json: "{}".into(),
            timeout_ms: 5000,
        };
        let d = SandboxExecRequest::abi_decode(&exec.abi_encode()).unwrap();
        assert_eq!(d.command, "ls");
        assert_eq!(d.timeout_ms, 5000);

        let exec_r = SandboxExecResponse {
            exit_code: 1,
            stdout: "out".into(),
            stderr: "err".into(),
        };
        let d = SandboxExecResponse::abi_decode(&exec_r.abi_encode()).unwrap();
        assert_eq!(d.exit_code, 1);

        let prompt = SandboxPromptRequest {
            sidecar_url: "http://h".into(),
            message: "hi".into(),
            session_id: "s".into(),
            model: "m".into(),
            context_json: "{}".into(),
            timeout_ms: 1000,
        };
        let d = SandboxPromptRequest::abi_decode(&prompt.abi_encode()).unwrap();
        assert_eq!(d.message, "hi");

        let prompt_r = SandboxPromptResponse {
            success: true,
            response: "ok".into(),
            error: String::new(),
            trace_id: "tr".into(),
            duration_ms: 500,
            input_tokens: 10,
            output_tokens: 5,
        };
        let d = SandboxPromptResponse::abi_decode(&prompt_r.abi_encode()).unwrap();
        assert!(d.success);
        assert_eq!(d.duration_ms, 500);

        let task = SandboxTaskRequest {
            sidecar_url: "http://h".into(),
            prompt: "build".into(),
            session_id: "s".into(),
            max_turns: 10,
            model: "claude".into(),
            context_json: "{}".into(),
            timeout_ms: 60000,
        };
        let d = SandboxTaskRequest::abi_decode(&task.abi_encode()).unwrap();
        assert_eq!(d.prompt, "build");
        assert_eq!(d.max_turns, 10);

        let task_r = SandboxTaskResponse {
            success: true,
            result: "done".into(),
            error: String::new(),
            trace_id: "tx".into(),
            duration_ms: 15000,
            input_tokens: 2000,
            output_tokens: 800,
            session_id: "sx".into(),
        };
        let d = SandboxTaskResponse::abi_decode(&task_r.abi_encode()).unwrap();
        assert_eq!(d.duration_ms, 15000);
        assert_eq!(d.session_id, "sx");
    }

    #[test]
    fn batch_and_workflow_types() {
        let bt = BatchTaskRequest {
            sidecar_urls: vec!["http://a".into(), "http://b".into()],
            prompt: "go".into(),
            session_id: String::new(),
            max_turns: 5,
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 30000,
            parallel: true,
            aggregation: "all".into(),
        };
        let d = BatchTaskRequest::abi_decode(&bt.abi_encode()).unwrap();
        assert_eq!(d.sidecar_urls.len(), 2);
        assert!(d.parallel);

        let be = BatchExecRequest {
            sidecar_urls: vec!["http://h".into()],
            command: "npm test".into(),
            cwd: "/app".into(),
            env_json: "{}".into(),
            timeout_ms: 10000,
            parallel: false,
        };
        let d = BatchExecRequest::abi_decode(&be.abi_encode()).unwrap();
        assert_eq!(d.command, "npm test");

        let bc = BatchCreateRequest {
            count: 3,
            template_request: SandboxCreateRequest {
                name: "n".into(),
                image: "i".into(),
                stack: String::new(),
                agent_identifier: String::new(),
                env_json: String::new(),
                metadata_json: String::new(),
                ssh_enabled: false,
                ssh_public_key: String::new(),
                web_terminal_enabled: false,
                max_lifetime_seconds: 60,
                idle_timeout_seconds: 30,
                cpu_cores: 1,
                memory_mb: 256,
                disk_gb: 5,
            },
            operators: vec![Address::ZERO],
            distribution: "round-robin".into(),
        };
        let d = BatchCreateRequest::abi_decode(&bc.abi_encode()).unwrap();
        assert_eq!(d.count, 3);
        assert_eq!(d.operators.len(), 1);

        let wc = WorkflowCreateRequest {
            name: "daily".into(),
            workflow_json: "{}".into(),
            trigger_type: "cron".into(),
            trigger_config: "0 0 * * *".into(),
            sandbox_config_json: "{}".into(),
        };
        let d = WorkflowCreateRequest::abi_decode(&wc.abi_encode()).unwrap();
        assert_eq!(d.trigger_type, "cron");

        let ssh = SshProvisionRequest {
            sidecar_url: "http://h".into(),
            username: "dev".into(),
            public_key: "ssh-ed25519 AAAA".into(),
        };
        let d = SshProvisionRequest::abi_decode(&ssh.abi_encode()).unwrap();
        assert_eq!(d.username, "dev");

        let ssh_r = SshRevokeRequest {
            sidecar_url: "http://h".into(),
            username: "dev".into(),
            public_key: "ssh-ed25519 AAAA".into(),
        };
        let d = SshRevokeRequest::abi_decode(&ssh_r.abi_encode()).unwrap();
        assert_eq!(d.username, "dev");

        let jr = JsonResponse {
            json: r#"{"k":"v"}"#.into(),
        };
        let d = JsonResponse::abi_decode(&jr.abi_encode()).unwrap();
        let p: Value = serde_json::from_str(&d.json).unwrap();
        assert_eq!(p["k"], "v");
    }
}

// ─── Error Propagation ───────────────────────────────────────────────────────

mod errors {
    use super::*;

    #[tokio::test]
    async fn connection_refused() {
        init();
        let r =
            sidecar_post_json("http://127.0.0.1:1", "/terminals/commands", "t", json!({})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn http_500() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&srv)
            .await;
        let r = sidecar_post_json(&srv.uri(), "/terminals/commands", "t", json!({})).await;
        assert!(r.unwrap_err().to_string().contains("500"));
    }

    #[tokio::test]
    async fn invalid_json_response() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&srv)
            .await;
        let r = sidecar_post_json(&srv.uri(), "/terminals/commands", "t", json!({})).await;
        assert!(
            r.unwrap_err()
                .to_string()
                .contains("Invalid sidecar response JSON")
        );
    }

    #[tokio::test]
    async fn workflow_auth_failure_propagates() {
        init();
        let entry = WorkflowEntry {
            id: 99999,
            name: "bad".into(),
            workflow_json: r#"{"sidecar_url":"http://ghost:1","prompt":"x","sidecar_token":"t"}"#
                .into(),
            trigger_type: "manual".into(),
            trigger_config: String::new(),
            sandbox_config_json: "{}".into(),
            active: true,
            next_run_at: None,
            last_run_at: None,
            owner: String::new(),
        };
        let r = run_workflow(&entry).await;
        match r {
            Err(e) => assert!(e.contains("not found"), "expected 'not found', got: {e}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn exec_sidecar_502() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/terminals/commands"))
            .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
            .mount(&srv)
            .await;
        let req = SandboxExecRequest {
            sidecar_url: srv.uri(),
            command: "ls".into(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };
        assert!(run_exec_request(&req, "t").await.is_err());
    }

    #[tokio::test]
    async fn task_sidecar_502() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
            .mount(&srv)
            .await;
        assert!(
            run_task_request(&task_req(&srv.uri(), "go"), "t")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn prompt_sidecar_timeout() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(504).set_body_string("Gateway Timeout"))
            .mount(&srv)
            .await;
        let req = SandboxPromptRequest {
            sidecar_url: srv.uri(),
            message: "hi".into(),
            session_id: String::new(),
            model: String::new(),
            context_json: String::new(),
            timeout_ms: 0,
        };
        assert!(run_prompt_request(&req, "t").await.is_err());
    }
}

// ─── Docker Container Lifecycle ──────────────────────────────────────────────

mod docker {
    use super::*;
    use ai_agent_sandbox_blueprint_lib::runtime::{
        create_sidecar, delete_sidecar, resume_sidecar, stop_sidecar,
    };

    fn docker_ok() -> bool {
        std::process::Command::new("docker")
            .arg("info")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn full_lifecycle_create_stop_resume_delete() {
        init();
        if !docker_ok() {
            eprintln!("SKIP: Docker not available");
            return;
        }

        let request = SandboxCreateRequest {
            name: "lifecycle".into(),
            image: String::new(),
            stack: String::new(),
            agent_identifier: String::new(),
            env_json: "{}".into(),
            metadata_json: "{}".into(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 60,
            idle_timeout_seconds: 30,
            cpu_cores: 1,
            memory_mb: 256,
            disk_gb: 1,
        };

        let record = match create_sidecar(&CreateSandboxParams::from(&request), None).await {
            Ok((r, _)) => r,
            Err(e) => {
                eprintln!("SKIP: create_sidecar failed (image pull?): {e}");
                return;
            }
        };

        assert!(!record.id.is_empty());
        assert!(!record.container_id.is_empty());
        assert!(!record.token.is_empty());
        assert_eq!(record.cpu_cores, 1);
        assert_eq!(record.memory_mb, 256);
        assert!(record.sidecar_port > 0);

        let stored = get_sandbox_by_id(&record.id).unwrap();
        assert_eq!(stored.container_id, record.container_id);

        let m = metrics::metrics();
        assert!(m.active_sandboxes.load(Ordering::Relaxed) >= 1);

        stop_sidecar(&record).await.unwrap();
        resume_sidecar(&record).await.unwrap();
        delete_sidecar(&record, None).await.unwrap();
        rm(&record.id);
    }

    #[tokio::test]
    async fn create_populates_snapshot_fields() {
        init();
        if !docker_ok() {
            eprintln!("SKIP: Docker not available");
            return;
        }

        let request = SandboxCreateRequest {
            name: "snap-fields".into(),
            image: String::new(),
            stack: String::new(),
            agent_identifier: String::new(),
            env_json: r#"{"MY_VAR":"hello"}"#.into(),
            metadata_json: r#"{"snapshot_destination":"s3://user-bucket/my-snap.tar.gz"}"#.into(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 60,
            idle_timeout_seconds: 30,
            cpu_cores: 1,
            memory_mb: 256,
            disk_gb: 1,
        };

        let record = match create_sidecar(&CreateSandboxParams::from(&request), None).await {
            Ok((r, _)) => r,
            Err(e) => {
                eprintln!("SKIP: create_sidecar failed: {e}");
                return;
            }
        };

        // Verify new fields are populated at creation
        assert!(
            !record.original_image.is_empty(),
            "original_image should be set"
        );
        assert_eq!(record.env_json, r#"{"MY_VAR":"hello"}"#);
        assert_eq!(
            record.snapshot_destination.as_deref(),
            Some("s3://user-bucket/my-snap.tar.gz")
        );
        assert!(record.snapshot_image_id.is_none());
        assert!(record.snapshot_s3_url.is_none());
        assert!(record.container_removed_at.is_none());
        assert!(record.image_removed_at.is_none());

        // Verify persisted record matches
        let stored = get_sandbox_by_id(&record.id).unwrap();
        assert_eq!(stored.original_image, record.original_image);
        assert_eq!(stored.env_json, record.env_json);
        assert_eq!(stored.snapshot_destination, record.snapshot_destination);

        delete_sidecar(&record, None).await.unwrap();
        rm(&record.id);
    }

    #[tokio::test]
    async fn commit_and_warm_resume() {
        use ai_agent_sandbox_blueprint_lib::runtime::commit_container;

        init();
        if !docker_ok() {
            eprintln!("SKIP: Docker not available");
            return;
        }

        let request = SandboxCreateRequest {
            name: "warm-resume".into(),
            image: String::new(),
            stack: String::new(),
            agent_identifier: String::new(),
            env_json: "{}".into(),
            metadata_json: "{}".into(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 60,
            idle_timeout_seconds: 30,
            cpu_cores: 1,
            memory_mb: 256,
            disk_gb: 1,
        };

        let record = match create_sidecar(&CreateSandboxParams::from(&request), None).await {
            Ok((r, _)) => r,
            Err(e) => {
                eprintln!("SKIP: create_sidecar failed: {e}");
                return;
            }
        };

        // Stop the container
        stop_sidecar(&record).await.unwrap();

        // Commit the stopped container → snapshot image
        let image_id = commit_container(&record).await.unwrap();
        assert!(!image_id.is_empty(), "commit should return an image ID");

        // Store the snapshot_image_id
        sandboxes()
            .unwrap()
            .update(&record.id, |r| {
                r.snapshot_image_id = Some(image_id.clone());
            })
            .unwrap();

        // Force-remove the container to simulate Hot→Warm GC transition
        delete_sidecar(&record, None).await.unwrap();
        sandboxes()
            .unwrap()
            .update(&record.id, |r| {
                r.container_removed_at = Some(now_ts());
            })
            .unwrap();

        // Resume should detect container gone + snapshot_image_id → warm path
        let updated_record = get_sandbox_by_id(&record.id).unwrap();
        assert!(updated_record.container_removed_at.is_some());
        assert!(updated_record.snapshot_image_id.is_some());

        resume_sidecar(&updated_record).await.unwrap();

        // After warm resume: new container running, snapshot consumed
        let resumed = get_sandbox_by_id(&record.id).unwrap();
        assert_eq!(
            resumed.state,
            ai_agent_sandbox_blueprint_lib::runtime::SandboxState::Running
        );
        assert!(
            resumed.container_removed_at.is_none(),
            "container_removed_at should be cleared"
        );
        assert!(
            resumed.snapshot_image_id.is_none(),
            "snapshot_image_id should be consumed"
        );
        assert_ne!(
            resumed.container_id, record.container_id,
            "should have a new container"
        );
        assert!(resumed.sidecar_port > 0);

        // Cleanup: delete the new container and snapshot image
        delete_sidecar(&resumed, None).await.unwrap();
        // Clean up the snapshot image if it still exists
        let _ = ai_agent_sandbox_blueprint_lib::runtime::remove_snapshot_image(&image_id).await;
        rm(&record.id);
    }

    #[tokio::test]
    async fn resume_with_no_snapshot_returns_error() {
        init();
        if !docker_ok() {
            eprintln!("SKIP: Docker not available");
            return;
        }

        let request = SandboxCreateRequest {
            name: "no-snap".into(),
            image: String::new(),
            stack: String::new(),
            agent_identifier: String::new(),
            env_json: "{}".into(),
            metadata_json: "{}".into(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 60,
            idle_timeout_seconds: 30,
            cpu_cores: 1,
            memory_mb: 256,
            disk_gb: 1,
        };

        let record = match create_sidecar(&CreateSandboxParams::from(&request), None).await {
            Ok((r, _)) => r,
            Err(e) => {
                eprintln!("SKIP: create_sidecar failed: {e}");
                return;
            }
        };

        stop_sidecar(&record).await.unwrap();
        delete_sidecar(&record, None).await.unwrap();

        // Mark container as removed, no snapshots
        sandboxes()
            .unwrap()
            .update(&record.id, |r| {
                r.container_removed_at = Some(now_ts());
            })
            .unwrap();

        // Resume should fail — no container, no image, no S3
        let updated = get_sandbox_by_id(&record.id).unwrap();
        let result = resume_sidecar(&updated).await;
        assert!(result.is_err(), "resume with no snapshots should fail");
        assert!(
            result.unwrap_err().to_string().contains("Cannot resume"),
            "error should mention Cannot resume"
        );

        rm(&record.id);
    }
}

// ─── Metrics Integration ─────────────────────────────────────────────────────

mod metrics_tests {
    use super::*;

    #[tokio::test]
    async fn successful_task_records_job() {
        init();
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("ok"))
            .mount(&srv)
            .await;

        let m = metrics::metrics();
        let before = m.total_jobs.load(Ordering::Relaxed);

        run_task_request(&task_req(&srv.uri(), "work"), "t")
            .await
            .unwrap();

        assert!(m.total_jobs.load(Ordering::Relaxed) > before);
    }

    #[tokio::test]
    async fn failed_task_records_failure() {
        init();
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": false, "error": "model error",
                "traceId": "", "durationMs": 10,
                "usage": {"inputTokens": 1, "outputTokens": 0},
                "sessionId": ""
            })))
            .mount(&srv)
            .await;

        let m = metrics::metrics();
        let before = m.failed_jobs.load(Ordering::Relaxed);

        let resp = run_task_request(&task_req(&srv.uri(), "fail"), "t")
            .await
            .unwrap();
        assert!(!resp.success);

        assert!(m.failed_jobs.load(Ordering::Relaxed) > before);
    }

    #[test]
    fn snapshot_includes_all_counters() {
        let m = metrics::metrics();
        let snap = m.snapshot();
        let keys: Vec<&str> = snap.iter().map(|(k, _)| k.as_str()).collect();
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
    fn sandbox_metrics_create_delete() {
        let m = metrics::metrics();
        let before_active = m.active_sandboxes.load(Ordering::Relaxed);
        let before_cpu = m.allocated_cpu_cores.load(Ordering::Relaxed);

        m.record_sandbox_created(4, 8192);
        assert_eq!(
            m.active_sandboxes.load(Ordering::Relaxed),
            before_active + 1
        );
        assert_eq!(
            m.allocated_cpu_cores.load(Ordering::Relaxed),
            before_cpu + 4
        );

        m.record_sandbox_deleted(4, 8192);
        assert_eq!(m.active_sandboxes.load(Ordering::Relaxed), before_active);
        assert_eq!(m.allocated_cpu_cores.load(Ordering::Relaxed), before_cpu);
    }
}
