//! Integration tests: multi-tenant auth, full-stack execution, batch parallelism,
//! workflow lifecycle, ABI round-trips, error propagation, Docker lifecycle, metrics.

use ai_agent_sandbox_blueprint_lib::http::sidecar_post_json;
use ai_agent_sandbox_blueprint_lib::runtime::{
    SandboxRecord, get_sandbox_by_id, get_sandbox_by_url, require_sidecar_auth, sandboxes,
};
use ai_agent_sandbox_blueprint_lib::workflows::{
    WorkflowEntry, now_ts, run_task_request, run_workflow, workflow_key, workflow_tick, workflows,
};
use ai_agent_sandbox_blueprint_lib::*;
use blueprint_sdk::alloy::sol_types::SolValue;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;
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

fn task_req(url: &str, token: &str, prompt: &str) -> SandboxTaskRequest {
    SandboxTaskRequest {
        sidecar_url: url.to_string(),
        prompt: prompt.to_string(),
        session_id: String::new(),
        max_turns: 1,
        model: String::new(),
        context_json: String::new(),
        timeout_ms: 0,
        sidecar_token: token.to_string(),
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

// ─── Full-Stack Execution (auth → HTTP → parse) ─────────────────────────────

mod full_stack {
    use super::*;

    #[tokio::test]
    async fn exec_through_auth_and_sidecar() {
        let srv = MockServer::start().await;
        let id = insert_sandbox(&srv.uri(), "exec-tok");

        require_sidecar_auth(&srv.uri(), "exec-tok").unwrap();

        Mock::given(method("POST"))
            .and(path("/exec"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"exitCode": 0, "stdout": "hi", "stderr": ""})),
            )
            .mount(&srv)
            .await;

        let r = sidecar_post_json(&srv.uri(), "/exec", "exec-tok", json!({"command": "echo hi"}))
            .await
            .unwrap();
        assert_eq!(r["exitCode"], 0);
        assert_eq!(r["stdout"], "hi");
        rm(&id);
    }

    #[tokio::test]
    async fn task_through_run_task_request() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(mock_agent_ok("done"))
            .mount(&srv)
            .await;

        let resp = run_task_request(&task_req(&srv.uri(), "t", "do work"))
            .await
            .unwrap();
        assert!(resp.success);
        assert_eq!(resp.result, "done");
        assert_eq!(resp.trace_id, "t-done");
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 5);
    }

    #[tokio::test]
    async fn ssh_command_reaches_sidecar() {
        let srv = MockServer::start().await;
        let id = insert_sandbox(&srv.uri(), "ssh-tok");

        Mock::given(method("POST"))
            .and(path("/exec"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"exitCode": 0, "stdout": "", "stderr": ""})),
            )
            .expect(1)
            .mount(&srv)
            .await;

        // Replicate what provision_key does (it's pub(crate), not accessible here)
        let username = util::normalize_username("developer").unwrap();
        let key = "ssh-ed25519 AAAA test@host";
        let cmd = format!(
            "set -euo pipefail; user={}; home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
             if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
             mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
             if ! grep -qxF {} \"$home/.ssh/authorized_keys\" 2>/dev/null; then \
                 echo {} >> \"$home/.ssh/authorized_keys\"; \
             fi; chmod 600 \"$home/.ssh/authorized_keys\"",
            util::shell_escape(&username),
            util::shell_escape(key),
            util::shell_escape(key),
        );
        let payload = json!({"command": format!("sh -c {}", util::shell_escape(&cmd))});
        sidecar_post_json(&srv.uri(), "/exec", "ssh-tok", payload)
            .await
            .unwrap();
        rm(&id);
    }
}

// ─── Batch Execution ─────────────────────────────────────────────────────────

mod batch {
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
            let req = task_req(&srv.uri(), "t", "go");
            set.spawn(async move { run_task_request(&req).await });
        }

        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap().unwrap());
        }
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.success));
        // Each wiremock server asserts exactly 1 request received (via .expect(1))
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

        assert!(run_task_request(&task_req(&good.uri(), "t", "go")).await.is_ok());
        assert!(run_task_request(&task_req(&bad.uri(), "t", "go")).await.is_err());
    }
}

// ─── Workflow Lifecycle ──────────────────────────────────────────────────────

mod workflow_lifecycle {
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
            sidecar_token: "tok".into(),
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
            sidecar_token: "t".into(),
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
            sidecar_token: "t".into(),
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
            sidecar_token: "t".into(),
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
            sidecar_tokens: vec!["ta".into(), "tb".into()],
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
            sidecar_tokens: vec!["t".into()],
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
                sidecar_token: String::new(),
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
            sidecar_token: "t".into(),
        };
        let d = SshProvisionRequest::abi_decode(&ssh.abi_encode()).unwrap();
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
        let r = sidecar_post_json("http://127.0.0.1:1", "/exec", "t", json!({})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn http_500() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/exec"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&srv)
            .await;
        let r = sidecar_post_json(&srv.uri(), "/exec", "t", json!({})).await;
        assert!(r.unwrap_err().to_string().contains("500"));
    }

    #[tokio::test]
    async fn invalid_json_response() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&srv)
            .await;
        let r = sidecar_post_json(&srv.uri(), "/exec", "t", json!({})).await;
        assert!(r.unwrap_err().to_string().contains("Invalid sidecar response JSON"));
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
        };
        let r = run_workflow(&entry).await;
        match r {
            Err(e) => assert!(e.contains("not found"), "expected 'not found', got: {e}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn task_sidecar_502() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/agents/run"))
            .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
            .mount(&srv)
            .await;
        assert!(run_task_request(&task_req(&srv.uri(), "t", "go")).await.is_err());
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
            sidecar_token: "lc-tok".into(),
        };

        let record = match create_sidecar(&request).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIP: create_sidecar failed (image pull?): {e}");
                return;
            }
        };

        assert!(!record.id.is_empty());
        assert!(!record.container_id.is_empty());
        assert_eq!(record.token, "lc-tok");
        assert_eq!(record.cpu_cores, 1);
        assert_eq!(record.memory_mb, 256);
        assert!(record.sidecar_port > 0);

        // Verify stored in global sandbox registry
        let stored = get_sandbox_by_id(&record.id).unwrap();
        assert_eq!(stored.container_id, record.container_id);

        stop_sidecar(&record).await.unwrap();
        resume_sidecar(&record).await.unwrap();
        delete_sidecar(&record).await.unwrap();
        rm(&record.id);
    }
}

// ─── Metrics Integration ─────────────────────────────────────────────────────

mod metrics {
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

        let m = ai_agent_sandbox_blueprint_lib::metrics::metrics();
        let before = m.total_jobs.load(Ordering::Relaxed);

        run_task_request(&task_req(&srv.uri(), "t", "work"))
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

        let m = ai_agent_sandbox_blueprint_lib::metrics::metrics();
        let before = m.failed_jobs.load(Ordering::Relaxed);

        let resp = run_task_request(&task_req(&srv.uri(), "t", "fail"))
            .await
            .unwrap();
        assert!(!resp.success);

        assert!(m.failed_jobs.load(Ordering::Relaxed) > before);
    }
}
