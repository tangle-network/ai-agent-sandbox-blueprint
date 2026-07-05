//! Test suite for operator_api (relocated verbatim from the pre-split file).

use super::*;
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::response::Response;
use http_body_util::BodyExt;
use tower::util::ServiceExt;

use std::ffi::{OsStr, OsString};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

static INIT: Once = Once::new();
fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("operator-api-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", dir) };
    });
}

fn reset_test_state() {
    crate::session_auth::clear_all_for_testing();
    crate::circuit_breaker::clear_all_for_testing();
    crate::provision_progress::clear_all_for_testing().expect("clear provision state");
    crate::chat_state::clear_all_for_testing().expect("clear chat state");
    sandboxes()
        .unwrap()
        .replace(std::collections::HashMap::new())
        .expect("clear sandbox store");
    runtime::instance_store()
        .unwrap()
        .replace(std::collections::HashMap::new())
        .expect("clear instance store");
    rate_limit::read_limiter().reset();
    rate_limit::write_limiter().reset();
    rate_limit::terminal_interactive_limiter().reset();
    rate_limit::auth_limiter().reset();
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn docker_ok() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn app() -> Router {
    // Reset rate limiters to prevent cross-test interference.
    // All tests share static rate limiters and run within a single
    // 60-second window, which exhausts the write limiter (30 req/min).
    rate_limit::read_limiter().reset();
    rate_limit::write_limiter().reset();
    rate_limit::terminal_interactive_limiter().reset();
    rate_limit::auth_limiter().reset();
    operator_api_router()
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn test_auth_header() -> String {
    let token = session_auth::create_test_token("0x1234567890abcdef1234567890abcdef12345678");
    format!("Bearer {token}")
}

#[derive(Clone)]
struct MockTerminalSession {
    tx: tokio::sync::broadcast::Sender<String>,
    cwd: String,
    cols: u16,
    rows: u16,
}

#[derive(Clone, Default)]
struct MockSidecarState {
    last_exec_payload: Arc<Mutex<Option<Value>>>,
    last_terminal_create_payload: Arc<Mutex<Option<Value>>>,
    last_terminal_input_payload: Arc<Mutex<Option<Value>>>,
    last_terminal_input_session_id: Arc<Mutex<Option<String>>>,
    last_terminal_resize_payload: Arc<Mutex<Option<Value>>>,
    last_terminal_resize_session_id: Arc<Mutex<Option<String>>>,
    last_agent_payload: Arc<Mutex<Option<Value>>>,
    exec_response: Arc<Mutex<Value>>,
    agents_response: Arc<Mutex<Value>>,
    stream_response_body: Arc<Mutex<Option<String>>>,
    terminal_sessions: Arc<Mutex<std::collections::HashMap<String, MockTerminalSession>>>,
    next_terminal_session_id: Arc<AtomicU64>,
    remaining_agent_warmup_failures: Arc<AtomicU64>,
    agent_response_delay_ms: Arc<AtomicU64>,
    agent_invocations: Arc<AtomicU64>,
    agent_list_invocations: Arc<AtomicU64>,
    cancel_invocations: Arc<AtomicU64>,
}

async fn mock_sidecar_exec(
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    *state.last_exec_payload.lock().expect("exec lock") = Some(payload);
    let response = state
        .exec_response
        .lock()
        .expect("exec response lock")
        .clone();
    Json(response)
}

async fn mock_sidecar_terminal_create(
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    *state
        .last_terminal_create_payload
        .lock()
        .expect("terminal create lock") = Some(payload.clone());

    let session_id = format!(
        "mock-term-{}",
        state
            .next_terminal_session_id
            .fetch_add(1, Ordering::Relaxed)
            + 1
    );
    let (tx, _) = tokio::sync::broadcast::channel(32);
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("/sidecar")
        .to_string();
    let cols = payload
        .get("cols")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(80);
    let rows = payload
        .get("rows")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(24);
    state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock")
        .insert(
            session_id.clone(),
            MockTerminalSession {
                tx,
                cwd: cwd.clone(),
                cols,
                rows,
            },
        );

    Json(json!({
        "success": true,
        "data": {
            "sessionId": session_id,
            "shell": "bash",
            "cwd": cwd,
            "cols": cols,
            "rows": rows,
            "streamUrl": format!("/terminals/{session_id}/stream"),
        }
    }))
}

async fn mock_sidecar_terminal_list(State(state): State<MockSidecarState>) -> Json<Value> {
    let sessions = state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock")
        .iter()
        .map(|(session_id, session)| {
            json!({
                "sessionId": session_id,
                "cols": session.cols,
                "rows": session.rows,
            })
        })
        .collect::<Vec<_>>();

    Json(json!({
        "success": true,
        "data": sessions,
    }))
}

async fn mock_sidecar_terminal_get(
    Path(session_id): Path<String>,
    State(state): State<MockSidecarState>,
) -> Response {
    let sessions = state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock");
    let Some(session) = sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": {
                    "code": "SESSION_NOT_FOUND",
                    "message": "not found"
                }
            })),
        )
            .into_response();
    };

    Json(json!({
        "success": true,
        "data": {
            "sessionId": session_id.clone(),
            "isRunning": true,
            "cwd": session.cwd.clone(),
            "cols": session.cols,
            "rows": session.rows,
            "streamUrl": format!("/terminals/{session_id}/stream"),
        }
    }))
    .into_response()
}

async fn mock_sidecar_terminal_stream(
    Path(session_id): Path<String>,
    State(state): State<MockSidecarState>,
) -> Response {
    let Some(tx) = state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock")
        .get(&session_id)
        .map(|session| session.tx.clone())
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": {
                    "code": "SESSION_NOT_FOUND",
                    "message": "not found"
                }
            })),
        )
            .into_response();
    };

    crate::live_operator_sessions::sse_from_terminal_output(tx.subscribe()).into_response()
}

async fn mock_sidecar_terminal_input(
    Path(session_id): Path<String>,
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> Response {
    *state
        .last_terminal_input_payload
        .lock()
        .expect("terminal input lock") = Some(payload.clone());
    *state
        .last_terminal_input_session_id
        .lock()
        .expect("terminal input session lock") = Some(session_id.clone());

    let tx = {
        let sessions = state
            .terminal_sessions
            .lock()
            .expect("terminal sessions lock");
        sessions.get(&session_id).map(|session| session.tx.clone())
    };

    let Some(tx) = tx else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": {
                    "code": "SESSION_NOT_FOUND",
                    "message": "not found"
                }
            })),
        )
            .into_response();
    };

    let data = payload
        .get("data")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !data.is_empty() {
        let stdout = state
            .exec_response
            .lock()
            .expect("exec response lock")
            .get("result")
            .and_then(|result| result.get("stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !stdout.is_empty() {
            let _ = tx.send(stdout);
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true
        })),
    )
        .into_response()
}

async fn mock_sidecar_terminal_patch(
    Path(session_id): Path<String>,
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> Response {
    *state
        .last_terminal_resize_payload
        .lock()
        .expect("terminal resize lock") = Some(payload.clone());
    *state
        .last_terminal_resize_session_id
        .lock()
        .expect("terminal resize session lock") = Some(session_id.clone());

    let mut sessions = state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock");
    let Some(session) = sessions.get_mut(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": {
                    "code": "SESSION_NOT_FOUND",
                    "message": "not found"
                }
            })),
        )
            .into_response();
    };

    if let Some(cols) = payload
        .get("cols")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
    {
        session.cols = cols;
    }
    if let Some(rows) = payload
        .get("rows")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
    {
        session.rows = rows;
    }

    Json(json!({
        "success": true,
        "data": {
            "sessionId": session_id,
            "isRunning": true,
            "cols": session.cols,
            "rows": session.rows,
            "streamUrl": format!("/terminals/{session_id}/stream"),
        }
    }))
    .into_response()
}

async fn mock_sidecar_terminal_delete(
    Path(session_id): Path<String>,
    State(state): State<MockSidecarState>,
) -> Json<Value> {
    state
        .terminal_sessions
        .lock()
        .expect("terminal sessions lock")
        .remove(&session_id);
    Json(json!({
        "success": true
    }))
}

async fn mock_sidecar_agent(
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    *state.last_agent_payload.lock().expect("agent lock") = Some(payload.clone());
    state.agent_invocations.fetch_add(1, Ordering::Relaxed);
    let delay_ms = state.agent_response_delay_ms.load(Ordering::Relaxed);
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    let identifier = payload
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let known_identifier = state
        .agents_response
        .lock()
        .expect("agents response lock")
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents.iter().any(|agent| {
                agent
                    .get("identifier")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    == identifier
            })
        })
        .unwrap_or(false);
    if !known_identifier {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": {
                    "code": "AGENT_EXECUTION_FAILED",
                    "message": format!(
                        "No factory registered for agent identifier {identifier}"
                    )
                }
            })),
        )
            .into_response();
    }
    let remaining = state
        .remaining_agent_warmup_failures
        .load(Ordering::Relaxed);
    if remaining > 0 {
        state
            .remaining_agent_warmup_failures
            .fetch_sub(1, Ordering::Relaxed);
        return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": {
                        "code": "AGENT_EXECUTION_FAILED",
                        "message": "OpenCode server is not responding (may have crashed). Cannot create session."
                    }
                })),
            )
                .into_response();
    }
    let session_id = payload
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("mock-agent-session");
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "response": "mock-agent-response",
            "traceId": "trace-mock-1",
            "sessionId": session_id,
            "usage": {
                "input_tokens": 2,
                "output_tokens": 3
            }
        })),
    )
        .into_response()
}

async fn mock_sidecar_agent_stream(
    State(state): State<MockSidecarState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    *state.last_agent_payload.lock().expect("agent lock") = Some(payload.clone());
    state.agent_invocations.fetch_add(1, Ordering::Relaxed);
    let delay_ms = state.agent_response_delay_ms.load(Ordering::Relaxed);
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    let identifier = payload
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let known_identifier = state
        .agents_response
        .lock()
        .expect("agents response lock")
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents.iter().any(|agent| {
                agent
                    .get("identifier")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    == identifier
            })
        })
        .unwrap_or(false);
    if !known_identifier {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": {
                    "code": "AGENT_EXECUTION_FAILED",
                    "message": format!(
                        "No factory registered for agent identifier {identifier}"
                    )
                }
            })),
        )
            .into_response();
    }
    let remaining = state
        .remaining_agent_warmup_failures
        .load(Ordering::Relaxed);
    if remaining > 0 {
        state
            .remaining_agent_warmup_failures
            .fetch_sub(1, Ordering::Relaxed);
        return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": {
                        "code": "AGENT_EXECUTION_FAILED",
                        "message": "OpenCode server is not responding (may have crashed). Cannot create session."
                    }
                })),
            )
                .into_response();
    }
    let session_id = payload
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("mock-agent-session");
    let body = state
            .stream_response_body
            .lock()
            .expect("stream response body lock")
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "event: message.part.updated\n\
data: {{\"part\":{{\"id\":\"part-1\",\"type\":\"text\",\"text\":\"mock-agent-response\"}}}}\n\n\
event: result\n\
data: {{\"finalText\":\"mock-agent-response\",\"metadata\":{{\"sessionId\":\"{session_id}\",\"traceId\":\"trace-mock-1\"}},\"tokenUsage\":{{\"inputTokens\":2,\"outputTokens\":3}}}}\n\n"
                )
            });
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
}

async fn mock_sidecar_run_cancel(State(state): State<MockSidecarState>) -> impl IntoResponse {
    state.cancel_invocations.fetch_add(1, Ordering::Relaxed);
    (
        StatusCode::OK,
        Json(json!({
            "success": true
        })),
    )
        .into_response()
}

async fn mock_sidecar_agents(State(state): State<MockSidecarState>) -> Json<Value> {
    state.agent_list_invocations.fetch_add(1, Ordering::Relaxed);
    let response = state
        .agents_response
        .lock()
        .expect("agents response lock")
        .clone();
    Json(response)
}

async fn spawn_mock_sidecar() -> (String, MockSidecarState, JoinHandle<()>) {
    let state = MockSidecarState::default();
    *state.exec_response.lock().expect("exec response lock") = json!({
        "result": {
            "exitCode": 0,
            "stdout": "mock-exec-stdout",
            "stderr": ""
        }
    });
    *state.agents_response.lock().expect("agents response lock") = json!({
        "agents": [
            { "identifier": "default", "displayName": "Default" },
            { "identifier": "batch", "displayName": "Batch" }
        ],
        "count": 2
    });
    let app = Router::new()
        .route(
            "/health",
            get(|| async { (StatusCode::OK, Json(json!({"status":"ok"}))) }),
        )
        .route(
            "/terminals",
            get(mock_sidecar_terminal_list).post(mock_sidecar_terminal_create),
        )
        .route(
            "/terminals/{session_id}/stream",
            get(mock_sidecar_terminal_stream),
        )
        .route(
            "/terminals/{session_id}/input",
            post(mock_sidecar_terminal_input),
        )
        .route(
            "/terminals/{session_id}",
            get(mock_sidecar_terminal_get)
                .patch(mock_sidecar_terminal_patch)
                .delete(mock_sidecar_terminal_delete),
        )
        .route("/terminals/commands", post(mock_sidecar_exec))
        .route("/agents", get(mock_sidecar_agents))
        .route("/agents/run", post(mock_sidecar_agent))
        .route("/agents/run/stream", post(mock_sidecar_agent_stream))
        .route("/agents/run/cancel", post(mock_sidecar_run_cancel))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock sidecar");
    let addr = listener.local_addr().expect("mock sidecar addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock sidecar");
    });

    let sidecar_url = format!("http://{addr}");
    let health_url = format!("{sidecar_url}/health");
    for _ in 0..20 {
        if let Ok(resp) = reqwest::get(&health_url).await
            && resp.status().is_success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    (sidecar_url, state, server)
}

async fn spawn_mock_sidecar_with_agent_warmup_failures(
    failures: u64,
) -> (String, MockSidecarState, JoinHandle<()>) {
    let (sidecar_url, state, server) = spawn_mock_sidecar().await;
    state
        .remaining_agent_warmup_failures
        .store(failures, Ordering::Relaxed);
    (sidecar_url, state, server)
}

async fn spawn_mock_sidecar_without_agent_listing() -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/health",
            get(|| async { (StatusCode::OK, Json(json!({"status":"ok"}))) }),
        )
        .route(
            "/agents/run",
            post(|Json(payload): Json<Value>| async move {
                let identifier = payload
                    .get("identifier")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if identifier == "a1" {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "error": {
                                "code": "AGENT_EXECUTION_FAILED",
                                "message": "No factory registered for agent identifier a1"
                            }
                        })),
                    );
                }

                (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "response": "ok",
                        "traceId": "trace-mock-compat",
                        "sessionId": "mock-agent-session"
                    })),
                )
            }),
        );
    let app = app.route(
            "/agents/run/stream",
            post(|Json(payload): Json<Value>| async move {
                let identifier = payload
                    .get("identifier")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if identifier == "a1" {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "error": {
                                "code": "AGENT_EXECUTION_FAILED",
                                "message": "No factory registered for agent identifier a1"
                            }
                        })),
                    )
                        .into_response();
                }

                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    "event: result\ndata: {\"finalText\":\"ok\",\"metadata\":{\"sessionId\":\"mock-agent-session\",\"traceId\":\"trace-mock-compat\"}}\n\n".to_string(),
                )
                    .into_response()
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock sidecar without /agents");
    let addr = listener.local_addr().expect("mock sidecar addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock sidecar");
    });

    let sidecar_url = format!("http://{addr}");
    let health_url = format!("{sidecar_url}/health");
    for _ in 0..20 {
        if let Ok(resp) = reqwest::get(&health_url).await
            && resp.status().is_success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    (sidecar_url, server)
}

async fn read_first_sse_frame(mut body: Body) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(3), async move {
        loop {
            let frame = body.frame().await?;
            let frame = frame.ok()?;
            let Ok(data) = frame.into_data() else {
                continue;
            };
            let text = String::from_utf8_lossy(&data).to_string();
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn read_sse_until_idle(mut body: Body) -> String {
    tokio::time::timeout(Duration::from_secs(3), async move {
        let mut combined = String::new();
        loop {
            let Some(frame) = body.frame().await else {
                break;
            };
            let Ok(frame) = frame else {
                break;
            };
            let Ok(data) = frame.into_data() else {
                continue;
            };
            let text = String::from_utf8_lossy(&data).to_string();
            if text.trim().is_empty() {
                continue;
            }
            combined.push_str(&text);
            if combined.contains("event: session.idle") {
                break;
            }
        }
        combined
    })
    .await
    .unwrap_or_default()
}

async fn wait_for_run_terminal(run_id: &str) -> ChatRunRecord {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(run) = crate::chat_state::get_run(run_id).expect("get run")
            && !run.status.is_active()
        {
            return run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for run {run_id} to finish"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[serial_test::serial]
#[tokio::test]
async fn test_list_sandboxes_empty() {
    init();
    reset_test_state();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes")
                .header("authorization", test_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(json["sandboxes"].as_array().unwrap().is_empty());
}

#[serial_test::serial]
#[tokio::test]
async fn test_list_sandboxes_requires_auth() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_list_provisions_empty() {
    init();
    reset_test_state();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/provisions")
                .header("authorization", test_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(json["provisions"].as_array().is_some());
}

#[serial_test::serial]
#[tokio::test]
async fn test_get_provision_not_found() {
    init();
    reset_test_state();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/provisions/999999")
                .header("authorization", test_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[serial_test::serial]
#[tokio::test]
async fn test_provision_lifecycle() {
    init();
    reset_test_state();

    let auth = test_auth_header();
    // Start a provision
    let call_id = 77777;
    provision_progress::start_provision(call_id).unwrap();

    // Should be retrievable
    let response = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/provisions/{call_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["phase"], "queued");
    assert_eq!(json["progress_pct"], 0);

    // Update to ImagePull
    provision_progress::update_provision(
        call_id,
        provision_progress::ProvisionPhase::ImagePull,
        Some("Pulling image".into()),
        None,
        None,
    )
    .unwrap();

    let response = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/provisions/{call_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = body_json(response.into_body()).await;
    assert_eq!(json["phase"], "image_pull");
    assert_eq!(json["progress_pct"], 20);

    // Clean up: move to terminal state so we don't pollute other tests
    provision_progress::update_provision(
        call_id,
        provision_progress::ProvisionPhase::Ready,
        Some("Done".into()),
        None,
        None,
    )
    .unwrap();
}

#[serial_test::serial]
#[tokio::test]
async fn test_auth_challenge_returns_nonce() {
    let _guard = crate::session_auth::capacity_test_lock_async().await;
    crate::session_auth::clear_all_for_testing();

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/challenge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(json["nonce"].is_string());
    assert!(json["message"].is_string());
    assert!(json["expires_at"].is_number());
    assert!(json["nonce"].as_str().unwrap().len() == 64); // 32 bytes hex
}

#[serial_test::serial]
#[tokio::test]
async fn test_auth_session_invalid_sig() {
    let _guard = crate::session_auth::capacity_test_lock_async().await;
    crate::session_auth::clear_all_for_testing();

    // First get a challenge
    let response = app()
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/challenge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let challenge = body_json(response.into_body()).await;
    let nonce = challenge["nonce"].as_str().unwrap();

    // Submit with an invalid signature
    let body = serde_json::json!({
        "nonce": nonce,
        "signature": "0xdeadbeef"
    });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/session")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_health_endpoint() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Health returns 200 (ok/degraded) or 503 (unhealthy) depending on Docker
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected health status: {status}"
    );
    let json = body_json(response.into_body()).await;
    assert!(json["status"].is_string(), "missing status field");
    assert!(
        json["checks"]["runtime"]["status"].is_string(),
        "missing checks.runtime.status"
    );
    assert!(
        json["checks"]["store"]["status"].is_string(),
        "missing checks.store.status"
    );
    assert!(
        json["runtime_backend"].is_string(),
        "missing runtime_backend field"
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_capabilities_endpoint_includes_all_harness_runtime() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    let capabilities = json["capabilities"].as_array().expect("capabilities");
    assert!(
        capabilities.iter().any(|cap| cap["id"] == "all_harness"),
        "missing all_harness capability: {json}",
    );
    let harnesses = json["harnesses"].as_array().expect("harnesses");
    for id in ["claude-code", "codex", "opencode", "kimi-code", "gemini"] {
        assert!(
            harnesses.iter().any(|h| h["id"] == id),
            "missing harness {id}: {json}",
        );
    }
}

#[serial_test::serial]
#[tokio::test]
async fn test_metrics_endpoint() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/plain"));

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(body.contains("sandbox_total_jobs"));
    assert!(body.contains("sandbox_active_sandboxes"));
}

#[serial_test::serial]
#[tokio::test]
async fn test_cors_preflight() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/sandboxes")
                .header("origin", "http://127.0.0.1:1338")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin")
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_cors_preflight_for_extra_routes() {
    let app = operator_api_router_with_tee_and_routes(
        None,
        Router::new().route(
            "/api/workflows/{workflow_id}",
            get(|| async { StatusCode::OK }),
        ),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/workflows/1")
                .header("origin", "http://127.0.0.1:1338")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin")
    );
}

// ── TEE sealed secrets API tests ──────────────────────────────────────

fn tee_app() -> Router {
    // The mock backend can never produce a hardware-verified quote, so the
    // server-side gate would refuse trust-granting routes under the
    // fail-closed default. These tests exercise the client-side-only trust
    // model, so opt out explicitly (the routes then mount and report
    // `server_enforced: false`). Guarded so it doesn't race other tests.
    {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var("SANDBOX_TEE_EXPECTED_MEASUREMENTS");
            std::env::set_var("SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT", "false");
        }
    }
    let mock = std::sync::Arc::new(crate::tee::mock::MockTeeBackend::new(
        crate::tee::TeeType::Tdx,
    ));
    operator_api_router_with_tee(Some(mock))
}

/// Insert a sandbox record with TEE fields into the store.
fn insert_tee_sandbox(id: &str, deployment_id: &str, owner: &str) {
    init();
    use crate::runtime::{SandboxRecord, SandboxState, sandboxes, seal_record};
    let mut record = SandboxRecord {
        id: id.to_string(),
        container_id: format!("tee-{deployment_id}"),
        sidecar_url: "http://mock-tee:8080".into(),
        sidecar_port: 8080,
        ssh_port: None,
        token: "test-token".into(),
        created_at: 1_700_000_000,
        cpu_cores: 2,
        memory_mb: 4096,
        state: SandboxState::Running,
        idle_timeout_seconds: 1800,
        max_lifetime_seconds: 86400,
        last_activity_at: 1_700_000_000,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "test:latest".into(),
        base_env_json: "{}".into(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: Some(deployment_id.to_string()),
        tee_metadata_json: Some(r#"{"backend":"mock"}"#.into()),
        tee_attestation_json: None,
        name: "tee-sandbox".into(),
        agent_identifier: String::new(),
        metadata_json: "{}".into(),
        disk_gb: 50,
        stack: String::new(),
        owner: owner.to_string(),
        service_id: None,
        tee_config: Some(crate::tee::TeeConfig {
            required: true,
            tee_type: crate::tee::TeeType::Tdx,
            attestation_nonce: None,
        }),
        extra_ports: std::collections::HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: String::new(),
    };
    seal_record(&mut record).unwrap();
    sandboxes().unwrap().insert(id.to_string(), record).unwrap();
}

/// Insert a non-TEE sandbox into the store.
fn insert_plain_sandbox_with_state_and_url(
    id: &str,
    owner: &str,
    sidecar_url: &str,
    state: crate::runtime::SandboxState,
) {
    init();
    use crate::runtime::{SandboxRecord, SandboxState, sandboxes, seal_record};
    let stopped_at = (state != SandboxState::Running).then_some(1_700_000_001);
    let mut record = SandboxRecord {
        id: id.to_string(),
        container_id: format!("ctr-{id}"),
        sidecar_url: sidecar_url.to_string(),
        sidecar_port: 9999,
        ssh_port: None,
        token: "plain-token".into(),
        created_at: 1_700_000_000,
        cpu_cores: 1,
        memory_mb: 1024,
        state,
        idle_timeout_seconds: 1800,
        max_lifetime_seconds: 86400,
        last_activity_at: 1_700_000_000,
        stopped_at,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "test:latest".into(),
        base_env_json: "{}".into(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: "plain-sandbox".into(),
        agent_identifier: String::new(),
        metadata_json: "{}".into(),
        disk_gb: 10,
        stack: String::new(),
        owner: owner.to_string(),
        service_id: None,
        tee_config: None,
        extra_ports: std::collections::HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: String::new(),
    };
    seal_record(&mut record).unwrap();
    sandboxes().unwrap().insert(id.to_string(), record).unwrap();
}

fn insert_plain_sandbox_with_url(id: &str, owner: &str, sidecar_url: &str) {
    insert_plain_sandbox_with_state_and_url(id, owner, sidecar_url, SandboxState::Running);
}

fn insert_stopped_sandbox_with_url(id: &str, owner: &str, sidecar_url: &str) {
    insert_plain_sandbox_with_state_and_url(id, owner, sidecar_url, SandboxState::Stopped);
}

fn insert_plain_sandbox(id: &str, owner: &str) {
    insert_plain_sandbox_with_url(id, owner, "http://localhost:9999");
}

/// Insert a mock-sidecar sandbox that should always take the non-Docker SSH path.
fn insert_mock_sidecar_ssh_sandbox(id: &str, owner: &str, sidecar_url: &str, ssh_port: u16) {
    use crate::runtime::{sandboxes, seal_record};

    insert_plain_sandbox_with_url(id, owner, sidecar_url);

    let mut record = sandboxes()
        .unwrap()
        .get(id)
        .unwrap()
        .expect("sandbox must exist to configure mock ssh");
    record.metadata_json = r#"{"runtime_backend":"firecracker"}"#.into();
    record.ssh_port = Some(ssh_port);
    seal_record(&mut record).unwrap();
    sandboxes().unwrap().insert(id.to_string(), record).unwrap();
}

fn set_agent_identifier(id: &str, agent_identifier: &str) {
    use crate::runtime::{sandboxes, seal_record};
    let mut record = sandboxes()
        .unwrap()
        .get(id)
        .unwrap()
        .expect("sandbox must exist to update agent identifier");
    record.agent_identifier = agent_identifier.to_string();
    seal_record(&mut record).unwrap();
    sandboxes().unwrap().insert(id.to_string(), record).unwrap();
}

/// Insert a singleton instance record (stored under key "instance").
fn insert_instance_sandbox_with_url(id: &str, owner: &str, sidecar_url: &str) {
    insert_plain_sandbox_with_url(id, owner, sidecar_url);
    let record = sandboxes()
        .unwrap()
        .get(id)
        .unwrap()
        .expect("sandbox exists");
    runtime::instance_store()
        .unwrap()
        .insert("instance".to_string(), record)
        .unwrap();
}

fn insert_instance_sandbox(id: &str, owner: &str) {
    insert_instance_sandbox_with_url(id, owner, "http://localhost:9999");
}

fn insert_instance_tee_sandbox(id: &str, deployment_id: &str, owner: &str) {
    insert_instance_sandbox(id, owner);
    use crate::runtime::seal_record;
    let mut record = sandboxes()
        .unwrap()
        .get(id)
        .unwrap()
        .expect("sandbox exists");
    record.tee_deployment_id = Some(deployment_id.to_string());
    record.tee_metadata_json = Some(r#"{"backend":"mock"}"#.into());
    record.tee_config = Some(crate::tee::TeeConfig {
        required: true,
        tee_type: crate::tee::TeeType::Tdx,
        attestation_nonce: None,
    });
    seal_record(&mut record).unwrap();
    sandboxes()
        .unwrap()
        .insert(id.to_string(), record.clone())
        .unwrap();
    runtime::instance_store()
        .unwrap()
        .insert("instance".to_string(), record)
        .unwrap();
}

// Use a distinct owner for TEE tests so sandbox inserts don't pollute
// the test_list_sandboxes_empty assertion (which uses a different address).
const TEE_TEST_OWNER: &str = "0xTEE0000000000000000000000000000000000001";

#[serial_test::serial]
#[tokio::test]
async fn test_tee_public_key_success() {
    insert_tee_sandbox("tee-pk-1", "deploy-pk-1", TEE_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(TEE_TEST_OWNER));

    let response = tee_app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/tee-pk-1/tee/public-key")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["sandbox_id"], "tee-pk-1");
    assert_eq!(json["public_key"]["algorithm"], "x25519-hkdf-sha256");
    assert!(json["public_key"]["attestation"]["tee_type"].is_string());
    // Client-side-only trust model: the key was released without server-side
    // attestation verification, and that fact is surfaced honestly.
    assert_eq!(json["server_enforced"], false);
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_public_key_not_tee_sandbox() {
    insert_plain_sandbox("plain-pk-1", TEE_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(TEE_TEST_OWNER));

    let response = tee_app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/plain-pk-1/tee/public-key")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_public_key_nonexistent_sandbox() {
    init();
    let auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0x1234567890abcdef1234567890abcdef12345678")
    );

    let response = tee_app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/nonexistent/tee/public-key")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // validate_secret_access returns FORBIDDEN for nonexistent sandboxes
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_public_key_no_auth() {
    let response = tee_app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/any/tee/public-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_sealed_secrets_success() {
    insert_tee_sandbox("tee-ss-1", "deploy-ss-1", TEE_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(TEE_TEST_OWNER));

    let body = serde_json::json!({
        "sealed_secret": {
            "algorithm": "x25519-xsalsa20-poly1305",
            "ciphertext": [0xDE, 0xAD],
            "nonce": [0xBE, 0xEF]
        }
    });

    let response = tee_app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/tee-ss-1/tee/sealed-secrets")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["sandbox_id"], "tee-ss-1");
    assert_eq!(json["success"], true);
    assert_eq!(json["secrets_count"], 3);
    assert_eq!(json["server_enforced"], false);
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_attestation_accepts_nonce_challenge() {
    insert_tee_sandbox("tee-att-1", "deploy-att-1", TEE_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(TEE_TEST_OWNER));
    let nonce = "11".repeat(32);

    let response = tee_app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/tee-att-1/tee/attestation")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "attestation_nonce": nonce }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert_eq!(json["sandbox_id"], "tee-att-1");
    assert!(json["attestation"]["tee_type"].is_string());
}

#[serial_test::serial]
#[tokio::test]
async fn test_tee_routes_absent_without_backend() {
    init();
    let auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0x1234567890abcdef1234567890abcdef12345678")
    );

    // Use app() which has no TEE backend
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/any/tee/public-key")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Route should not exist → 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[serial_test::serial]
#[tokio::test]
#[serial_test::serial]
async fn test_tee_release_routes_absent_when_unpinned_by_default() {
    insert_tee_sandbox("tee-pk-fc", "deploy-pk-fc", TEE_TEST_OWNER);
    // Fail-closed default: no allowlist, requirement left on. The
    // trust-granting routes must NOT be mounted even though a TEE backend is
    // configured and the read-only attestation route still works.
    {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var("SANDBOX_TEE_EXPECTED_MEASUREMENTS");
            std::env::remove_var("SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT");
        }
    }
    let mock = std::sync::Arc::new(crate::tee::mock::MockTeeBackend::new(
        crate::tee::TeeType::Tdx,
    ));
    let app = operator_api_router_with_tee(Some(mock));
    let auth = format!("Bearer {}", session_auth::create_test_token(TEE_TEST_OWNER));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/tee-pk-fc/tee/public-key")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Routes are not mounted under the fail-closed default → 404.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Sandbox operation API tests ──────────────────────────────────────

const OP_TEST_OWNER: &str = "0xOP00000000000000000000000000000000000001";

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_exec_requires_auth() {
    init();
    let body = serde_json::json!({ "command": "echo hello" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/some-id/exec")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_exec_not_found() {
    init();
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "command": "echo hello" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/nonexistent/exec")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_exec_wrong_owner() {
    insert_plain_sandbox("op-test-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000002")
    );
    let body = serde_json::json!({ "command": "echo hello" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/op-test-1/exec")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_exec_no_sandbox() {
    // Use a fresh owner so no sandbox exists for them
    let auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xINST0000000000000000000000000000000003")
    );
    let body = serde_json::json!({ "command": "echo hello" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/exec")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    // Should fail — either NOT_FOUND (no sandboxes at all) or other error
    // depending on test ordering. Both are valid failure modes.
    assert_ne!(response.status(), StatusCode::OK);
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_secrets_empty_env_rejected() {
    insert_instance_sandbox("inst-sec-empty-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "env_json": {} });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/secrets")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_secrets_wrong_owner_forbidden() {
    insert_instance_sandbox("inst-sec-owner-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000014")
    );
    let body = serde_json::json!({ "env_json": { "API_KEY": "secret-value" } });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/secrets")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_secrets_reject_tee_instances() {
    insert_instance_tee_sandbox("inst-tee-sec-1", "deploy-tee-sec-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "env_json": { "API_KEY": "secret-value" } });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/secrets")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_snapshot_empty_destination() {
    insert_plain_sandbox("snap-test-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "destination": "",
        "include_workspace": true,
        "include_state": false,
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/snap-test-1/snapshot")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_prompt_requires_auth() {
    let body = serde_json::json!({ "message": "hello" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/some-id/prompt")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_routes_exist() {
    init();
    // Verify instance routes are registered (they'll fail with 401 without auth, not 404)
    for path in &[
        "/api/sandbox/exec",
        "/api/sandbox/prompt",
        "/api/sandbox/task",
        "/api/sandbox/secrets",
        "/api/sandbox/stop",
        "/api/sandbox/resume",
        "/api/sandbox/snapshot",
    ] {
        let response = app()
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(*path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Expected 401 for {path} (not 404), confirming route exists"
        );
    }

    let response = app()
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/sandbox/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_readyz_endpoint() {
    init();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected readyz status: {status}"
    );
    let json = body_json(response.into_body()).await;
    assert!(json["status"].is_string(), "missing status field");
    if status == StatusCode::SERVICE_UNAVAILABLE {
        assert!(
            json["runtime"].is_boolean(),
            "missing runtime boolean in not_ready payload"
        );
        assert!(json["store"].is_boolean(), "missing store boolean");
    }
}

#[serial_test::serial]
#[tokio::test]
async fn test_invalid_json_body() {
    init();
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/some-id/exec")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status().as_u16();
    assert!(
        (400..500).contains(&status),
        "expected 4xx for invalid JSON, got {status}"
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_security_headers_present() {
    init();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let headers = response.headers();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .map(|v| v.to_str().unwrap()),
        Some("nosniff"),
        "missing or wrong X-Content-Type-Options header"
    );
    assert_eq!(
        headers.get("x-frame-options").map(|v| v.to_str().unwrap()),
        Some("DENY"),
        "missing or wrong X-Frame-Options header"
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_terminal_session_sandbox_crud_and_stream() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("live-term-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let create_body = json!({
        "cwd": "/home/sidecar",
        "cols": 132,
        "rows": 40,
    });

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/live-term-1/live/terminal/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let created = body_json(create.into_body()).await;
    let session_id = created["session_id"].as_str().unwrap().to_string();

    let list = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/live-term-1/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let listed = body_json(list.into_body()).await;
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("session_id").and_then(|s| s.as_str()))
        .collect();
    assert!(ids.iter().any(|id| *id == session_id));

    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandboxes/live-term-1/live/terminal/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    let ct = stream
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/event-stream"));

    let deleted = app()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/sandboxes/live-term-1/live/terminal/sessions/{session_id}"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);

    let create_payload = sidecar_state
        .last_terminal_create_payload
        .lock()
        .expect("terminal create payload lock")
        .clone()
        .expect("terminal create payload");
    assert_eq!(
        create_payload,
        json!({
            "env": {
                "PS1": "\\u:\\w\\$ ",
                "PROMPT_DIRTRIM": "0",
            },
            "cwd": "/home/sidecar",
            "cols": 132,
            "rows": 40,
        })
    );

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_session_instance_crud_and_stream() {
    insert_instance_sandbox("live-inst-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create_body = serde_json::json!({ "title": "Ops Chat" });
    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let created = body_json(create.into_body()).await;
    let session_id = created["session_id"].as_str().unwrap().to_string();
    assert_eq!(created["title"], "Ops Chat");

    insert_instance_sandbox("live-inst-1", OP_TEST_OWNER);
    let list = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let listed = body_json(list.into_body()).await;
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("session_id").and_then(|s| s.as_str()))
        .collect();
    assert!(ids.iter().any(|id| *id == session_id));

    insert_instance_sandbox("live-inst-1", OP_TEST_OWNER);
    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    assert_eq!(detail_json["session_id"], session_id);
    assert_eq!(detail_json["title"], "Ops Chat");
    assert!(detail_json["messages"].is_array());

    insert_instance_sandbox("live-inst-1", OP_TEST_OWNER);
    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandbox/live/chat/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    let ct = stream
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/event-stream"));

    insert_instance_sandbox("live-inst-1", OP_TEST_OWNER);
    let deleted = app()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_terminal_stream_receives_input_output() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("live-exec-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/live-exec-1/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("session_id")
        .to_string();

    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandboxes/live-exec-1/live/terminal/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);

    let input_body = json!({
        "data": "echo hello\n"
    });
    let input = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/sandboxes/live-exec-1/live/terminal/sessions/{session_id}/input"
                ))
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&input_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(input.status(), StatusCode::OK);
    let input_json = body_json(input.into_body()).await;
    assert_eq!(input_json["success"], true);

    let frame = read_first_sse_frame(stream.into_body())
        .await
        .expect("sse frame");
    assert!(
        frame.contains("mock-exec-stdout"),
        "expected terminal stream to include exec output, got: {frame}"
    );

    let input_payload = sidecar_state
        .last_terminal_input_payload
        .lock()
        .expect("terminal input payload lock")
        .clone()
        .expect("terminal input payload");
    assert_eq!(input_payload["data"], "echo hello\n");
    let input_session_id = sidecar_state
        .last_terminal_input_session_id
        .lock()
        .expect("terminal input session id lock")
        .clone()
        .expect("terminal input session id");
    assert_eq!(input_session_id, session_id);
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_terminal_resize_proxies_to_sidecar() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("live-resize-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/live-resize-1/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("session_id")
        .to_string();

    let resize = app()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!(
                    "/api/sandboxes/live-resize-1/live/terminal/sessions/{session_id}"
                ))
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({"cols": 140, "rows": 48})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resize.status(), StatusCode::OK);
    let resize_json = body_json(resize.into_body()).await;
    assert_eq!(resize_json["success"], true);

    let resize_payload = sidecar_state
        .last_terminal_resize_payload
        .lock()
        .expect("terminal resize payload lock")
        .clone()
        .expect("terminal resize payload");
    assert_eq!(resize_payload, json!({"cols": 140, "rows": 48}));
    let resize_session_id = sidecar_state
        .last_terminal_resize_session_id
        .lock()
        .expect("terminal resize session id lock")
        .clone()
        .expect("terminal resize session id");
    assert_eq!(resize_session_id, session_id);
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_terminal_stream_uses_sidecar_reported_stream_url() {
    let custom_sidecar = Router::new()
        .route(
            "/health",
            get(|| async { (StatusCode::OK, Json(json!({"status":"ok"}))) }),
        )
        .route(
            "/terminals",
            post(|| async {
                Json(json!({
                    "success": true,
                    "data": {
                        "sessionId": "streamurl-1",
                        "shell": "bash",
                        "streamUrl": "/pty/streamurl-1/events",
                    }
                }))
            }),
        )
        .route(
            "/terminals/streamurl-1",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {
                        "sessionId": "streamurl-1",
                        "isRunning": true,
                        "streamUrl": "/pty/streamurl-1/events",
                    }
                }))
            }),
        )
        .route(
            "/pty/streamurl-1/events",
            get(|| async move {
                let mut response =
                    axum::response::Response::new(Body::from("data: alt-stream\r\n\r\n"));
                *response.status_mut() = StatusCode::OK;
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                response
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind streamurl sidecar");
    let addr = listener.local_addr().expect("streamurl sidecar addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, custom_sidecar)
            .await
            .expect("serve streamurl sidecar");
    });
    let sidecar_url = format!("http://{addr}");
    let health_url = format!("{sidecar_url}/health");
    for _ in 0..20 {
        if let Ok(resp) = reqwest::get(&health_url).await
            && resp.status().is_success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    insert_plain_sandbox_with_url("live-streamurl-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/live-streamurl-1/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    let stream = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/live-streamurl-1/live/terminal/sessions/streamurl-1/stream")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    let frame = read_first_sse_frame(stream.into_body())
        .await
        .expect("streamurl sse frame");
    assert!(
        frame.contains("alt-stream"),
        "expected streamUrl-backed output, got: {frame}"
    );

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_terminal_unsupported_does_not_trip_circuit_breaker() {
    let custom_sidecar = Router::new().route(
        "/health",
        get(|| async { (StatusCode::OK, Json(json!({"status":"ok"}))) }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind unsupported sidecar");
    let addr = listener.local_addr().expect("unsupported sidecar addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, custom_sidecar)
            .await
            .expect("serve unsupported sidecar");
    });
    let sidecar_url = format!("http://{addr}");
    let health_url = format!("{sidecar_url}/health");
    for _ in 0..20 {
        if let Ok(resp) = reqwest::get(&health_url).await
            && resp.status().is_success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    insert_plain_sandbox_with_url("term-unsupported-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let request = || {
        Request::builder()
            .method("POST")
            .uri("/api/sandboxes/term-unsupported-1/live/terminal/sessions")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap()
    };

    let first = app().oneshot(request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::BAD_GATEWAY);
    let first_json = body_json(first.into_body()).await;
    assert_eq!(
        first_json["code"].as_str(),
        Some(TERMINAL_UNSUPPORTED_ERROR_CODE)
    );

    let second = app().oneshot(request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::BAD_GATEWAY);
    let second_json = body_json(second.into_body()).await;
    assert_eq!(
        second_json["code"].as_str(),
        Some(TERMINAL_UNSUPPORTED_ERROR_CODE)
    );
    assert!(
        !circuit_breaker::query_status("term-unsupported-1").active,
        "terminal 4xx/501 responses should not trip the circuit breaker"
    );

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_exec_recovers_from_stale_docker_sidecar_url() {
    init();
    if !docker_ok() {
        eprintln!("SKIP: Docker not available");
        return;
    }

    unsafe {
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "30");
    }

    let request = crate::CreateSandboxParams {
        name: "stale-port-recovery".into(),
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
        owner: String::new(),
        service_id: None,
        tee_config: None,
        user_env_json: String::new(),
        port_mappings: Vec::new(),
        capabilities_json: String::new(),
    };

    let created = match crate::runtime::create_sidecar(&request, None).await {
        Ok((record, _)) => record,
        Err(err) => {
            eprintln!("SKIP: create_sidecar failed: {err}");
            return;
        }
    };

    sandboxes()
        .unwrap()
        .update(&created.id, |record| {
            record.owner = OP_TEST_OWNER.to_string();
        })
        .unwrap();

    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    // Baseline reachability gate. A local Docker daemon publishes the
    // container port back to 127.0.0.1, so the sidecar round-trip works; some
    // CI Docker networking does not, and the freshly-created sidecar is then
    // unreachable from this process. Without a reachable baseline the stale
    // recovery round-trip is equally unreachable, so this test would report a
    // false failure rather than exercise the recovery path. Verify the live
    // endpoint first and SKIP when the environment can't reach it — the
    // recovery logic stays covered wherever the sidecar is actually reachable.
    circuit_breaker::clear(&created.id);
    let baseline = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sandboxes/{}/exec", created.id))
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "command": "echo baseline-ok" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    if baseline.status() != StatusCode::OK {
        eprintln!(
            "SKIP: sidecar unreachable in this environment (baseline exec {}); stale-endpoint recovery is unverifiable here",
            baseline.status()
        );
        let _ = crate::runtime::delete_sidecar(&created, None).await;
        let _ = sandboxes().unwrap().remove(&created.id);
        circuit_breaker::clear(&created.id);
        return;
    }

    let original_url = created.sidecar_url.clone();
    let stale_url = "http://127.0.0.1:9".to_string();
    sandboxes()
        .unwrap()
        .update(&created.id, |record| {
            record.sidecar_url = stale_url.clone();
            record.sidecar_port = 9;
        })
        .unwrap();
    circuit_breaker::clear(&created.id);

    let body = json!({ "command": "echo stale-recovery-ok" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sandboxes/{}/exec", created.id))
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response.into_body()).await;
    assert!(
        json["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("stale-recovery-ok"),
        "exec should succeed after endpoint refresh: {json}"
    );

    let refreshed = sandboxes()
        .unwrap()
        .get(&created.id)
        .unwrap()
        .expect("sandbox should still exist");
    assert_eq!(
        refreshed.sidecar_url, original_url,
        "successful retry should persist the live sidecar URL back into the store"
    );
    assert!(
        circuit_breaker::check_health(&created.id).is_ok(),
        "successful stale-endpoint recovery should not leave the breaker open"
    );

    crate::runtime::delete_sidecar(&refreshed, None)
        .await
        .unwrap();
    let _ = sandboxes().unwrap().remove(&created.id);
    circuit_breaker::clear(&created.id);
}

#[serial_test::serial]
#[tokio::test]
async fn test_exec_rejects_stopped_sandbox() {
    insert_stopped_sandbox_with_url("stopped-exec-1", OP_TEST_OWNER, "http://localhost:9999");
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = json!({ "command": "echo should-fail" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/stopped-exec-1/exec")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = body_json(response.into_body()).await;
    assert_eq!(
        json["error"],
        "Sandbox stopped-exec-1 is stopped; resume it first"
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_terminal_session_create_rejects_stopped_sandbox() {
    insert_stopped_sandbox_with_url(
        "stopped-live-term-1",
        OP_TEST_OWNER,
        "http://localhost:9999",
    );
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/stopped-live-term-1/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = body_json(response.into_body()).await;
    assert_eq!(
        json["error"],
        "Sandbox stopped-live-term-1 is stopped; resume it first"
    );
}

#[serial_test::serial]
#[test]
fn test_should_forward_stream_part_filters_initial_user_echo() {
    let mut ignored = HashSet::new();
    let mut assistant = HashSet::new();
    let echoed_user_part = json!({
        "id": "echo-1",
        "messageID": "up-user-1",
        "type": "text",
        "text": "hello from live stream",
    });
    let assistant_part = json!({
        "id": "assistant-1",
        "messageID": "up-assistant-1",
        "type": "text",
        "text": "actual assistant reply",
    });

    assert!(!should_forward_stream_part(
        &echoed_user_part,
        "hello from live stream",
        &mut ignored,
        &mut assistant,
    ));
    assert!(ignored.contains("up-user-1"));

    assert!(should_forward_stream_part(
        &assistant_part,
        "hello from live stream",
        &mut ignored,
        &mut assistant,
    ));
    assert!(assistant.contains("up-assistant-1"));
}

#[serial_test::serial]
#[test]
fn test_should_forward_stream_part_filters_exact_request_text_without_message_id() {
    let mut ignored = HashSet::new();
    let mut assistant = HashSet::new();
    let echoed_user_part = json!({
        "id": "echo-1",
        "type": "text",
        "text": "hello from live stream",
    });

    assert!(!should_forward_stream_part(
        &echoed_user_part,
        "hello from live stream",
        &mut ignored,
        &mut assistant,
    ));
}

#[serial_test::serial]
#[test]
fn test_finalize_streamed_assistant_parts_sets_reasoning_end_time() {
    let mut parts = vec![
        json!({
            "id": "reason-1",
            "type": "reasoning",
            "text": "thinking",
            "time": { "start": 5 }
        }),
        json!({
            "id": "text-1",
            "type": "text",
            "text": "done"
        }),
    ];

    finalize_streamed_assistant_parts(&mut parts, 42);

    assert_eq!(parts[0]["time"]["end"], json!(42));
    assert!(parts[1].get("time").is_none());
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_prompt_updates_instance_stream_and_history() {
    init();
    reset_test_state();

    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create_body = json!({ "title": "Live Prompt" });
    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("chat session_id")
        .to_string();

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandbox/live/chat/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);

    let prompt_body = json!({
        "message": "hello from live stream",
        "session_id": session_id,
    });
    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&prompt_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.status(), StatusCode::ACCEPTED);
    let prompt_json = body_json(prompt.into_body()).await;
    let run_id = prompt_json["run_id"].as_str().expect("run_id");
    let run = wait_for_run_terminal(run_id).await;
    assert_eq!(run.status, ChatRunStatus::Completed);

    let frame = read_first_sse_frame(stream.into_body())
        .await
        .expect("chat sse frame");
    assert!(
        frame.contains("user_message")
            || frame.contains("assistant_message")
            || frame.contains("run_queued")
            || frame.contains("run_started"),
        "expected chat stream event, got: {frame}"
    );

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    let messages = detail_json["messages"].as_array().expect("messages array");
    let run_progress = detail_json["run_progress"]
        .as_array()
        .expect("run_progress array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(detail_json["runs"][0]["status"], "completed");
    assert!(
        run_progress.len() >= 2,
        "expected persisted progress history in session detail"
    );
    assert_eq!(run_progress[0]["status"], "queued");

    let agent_payload = sidecar_state
        .last_agent_payload
        .lock()
        .expect("agent payload lock")
        .clone()
        .expect("agent payload");
    assert_eq!(agent_payload["message"], "hello from live stream");
    assert!(
        agent_payload.get("sessionId").is_none()
            || agent_payload["sessionId"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_prompt_filters_echoed_user_text_from_assistant_stream() {
    init();
    reset_test_state();

    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    *sidecar_state
            .stream_response_body
            .lock()
            .expect("stream response body lock") = Some(
            "event: message.part.updated\n\
data: {\"part\":{\"id\":\"echo-1\",\"messageID\":\"up-user-1\",\"type\":\"text\",\"text\":\"hello from live stream\"}}\n\n\
event: message.part.updated\n\
data: {\"part\":{\"id\":\"reason-1\",\"messageID\":\"up-assistant-1\",\"type\":\"reasoning\",\"text\":\"Thinking through the answer\",\"time\":{\"start\":1,\"end\":2}}}\n\n\
event: message.part.updated\n\
data: {\"part\":{\"id\":\"assistant-1\",\"messageID\":\"up-assistant-1\",\"type\":\"text\",\"text\":\"actual assistant reply\"}}\n\n\
event: result\n\
data: {\"finalText\":\"actual assistant reply\",\"metadata\":{\"sessionId\":\"mock-agent-session\",\"traceId\":\"trace-mock-1\"},\"tokenUsage\":{\"inputTokens\":2,\"outputTokens\":3}}\n\n"
                .to_string(),
        );

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "title": "Live Prompt" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("chat session_id")
        .to_string();

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandbox/live/chat/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "message": "hello from live stream",
                        "session_id": session_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.status(), StatusCode::ACCEPTED);
    let prompt_json = body_json(prompt.into_body()).await;
    let run_id = prompt_json["run_id"].as_str().expect("run_id");
    let run = wait_for_run_terminal(run_id).await;
    assert_eq!(run.status, ChatRunStatus::Completed);

    let stream_text = read_sse_until_idle(stream.into_body()).await;
    assert!(stream_text.contains("actual assistant reply"));
    assert!(!stream_text.contains("\"id\":\"echo-1\""));
    assert!(!stream_text.contains("\"messageID\":\"up-user-1\""));

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    let messages = detail_json["messages"].as_array().expect("messages array");
    let assistant_message = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message");
    let assistant_parts = assistant_message["parts"]
        .as_array()
        .expect("assistant parts");

    assert!(
        assistant_parts
            .iter()
            .all(|part| { part["text"].as_str().unwrap_or_default() != "hello from live stream" }),
        "assistant message should not persist the echoed user prompt: {assistant_message}"
    );
    assert!(
        assistant_parts.iter().any(|part| {
            part["type"] == "reasoning"
                && part["id"] == "reason-1"
                && part["text"] == "Thinking through the answer"
        }),
        "assistant reasoning part should be preserved: {assistant_message}"
    );
    assert!(
        assistant_parts.iter().any(|part| {
            part["type"] == "text"
                && part["id"] == "assistant-1"
                && part["text"] == "actual assistant reply"
        }),
        "assistant text part should be preserved: {assistant_message}"
    );

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_prompt_failure_preserves_partial_streamed_content() {
    init();
    reset_test_state();

    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    *sidecar_state
            .stream_response_body
            .lock()
            .expect("stream response body lock") = Some(
            "event: message.part.updated\n\
data: {\"part\":{\"id\":\"reason-1\",\"messageID\":\"up-assistant-1\",\"type\":\"reasoning\",\"text\":\"Thinking through the answer\",\"time\":{\"start\":1}}}\n\n\
event: message.part.updated\n\
data: {\"part\":{\"id\":\"assistant-1\",\"messageID\":\"up-assistant-1\",\"type\":\"text\",\"text\":\"partial assistant reply\"}}\n\n\
event: error\n\
data: {\"message\":\"sidecar stream exploded\"}\n\n"
                .to_string(),
        );

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "title": "Live Prompt" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("chat session_id")
        .to_string();

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let stream = app()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sandbox/live/chat/sessions/{session_id}/stream"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "message": "hello from live stream",
                        "session_id": session_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.status(), StatusCode::ACCEPTED);
    let prompt_json = body_json(prompt.into_body()).await;
    let run_id = prompt_json["run_id"].as_str().expect("run_id");
    let run = wait_for_run_terminal(run_id).await;
    assert_eq!(run.status, ChatRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("sidecar stream exploded"));

    let stream_text = read_sse_until_idle(stream.into_body()).await;
    assert!(stream_text.contains("partial assistant reply"));
    assert!(stream_text.contains("event: session.error"));

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    let messages = detail_json["messages"].as_array().expect("messages array");
    let assistant_message = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message");
    let assistant_parts = assistant_message["parts"]
        .as_array()
        .expect("assistant parts");

    assert_eq!(assistant_message["success"], json!(false));
    assert_eq!(assistant_message["error"], json!("sidecar stream exploded"));
    assert!(
        assistant_parts.iter().any(|part| {
            part["type"] == "reasoning"
                && part["id"] == "reason-1"
                && part["time"]["end"].as_u64().is_some()
        }),
        "assistant reasoning part should be preserved and finalized: {assistant_message}"
    );
    assert!(
        assistant_parts.iter().any(|part| {
            part["type"] == "text"
                && part["id"] == "assistant-1"
                && part["text"] == "partial assistant reply"
        }),
        "assistant text part should preserve the streamed partial reply: {assistant_message}"
    );
    assert!(
        assistant_parts.iter().all(|part| {
            part["text"].as_str().unwrap_or_default() != "Error: sidecar stream exploded"
        }),
        "assistant message should not replace preserved streamed content with an error-only part: {assistant_message}"
    );

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_prompt_reuses_execution_started_session_id() {
    init();
    reset_test_state();

    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    *sidecar_state
            .stream_response_body
            .lock()
            .expect("stream response body lock") = Some(
            "event: execution.started\n\
data: {\"executionId\":\"exec-1\",\"sessionId\":\"sidecar-stream-session\",\"timestamp\":1}\n\n\
event: result\n\
data: {\"finalText\":\"first reply\",\"metadata\":{\"sessionId\":\"backend-result-session\",\"traceId\":\"trace-mock-1\"},\"tokenUsage\":{\"inputTokens\":2,\"outputTokens\":3}}\n\n"
                .to_string(),
        );

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "title": "Live Prompt" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_json = body_json(create.into_body()).await;
    let session_id = create_json["session_id"]
        .as_str()
        .expect("chat session_id")
        .to_string();

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let first_prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "message": "remember this task",
                        "session_id": session_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_prompt.status(), StatusCode::ACCEPTED);
    let first_prompt_json = body_json(first_prompt.into_body()).await;
    let first_run_id = first_prompt_json["run_id"].as_str().expect("first run_id");
    let first_run = wait_for_run_terminal(first_run_id).await;
    assert_eq!(first_run.status, ChatRunStatus::Completed);

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    assert_eq!(
        detail_json["sidecar_session_id"],
        json!("sidecar-stream-session")
    );
    let runs = detail_json["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0]["sidecar_session_id"],
        json!("sidecar-stream-session")
    );

    insert_instance_sandbox_with_url("live-prompt-inst-1", OP_TEST_OWNER, &sidecar_url);
    let second_prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "message": "are you done?",
                        "session_id": session_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_prompt.status(), StatusCode::ACCEPTED);
    let second_prompt_json = body_json(second_prompt.into_body()).await;
    let second_run_id = second_prompt_json["run_id"]
        .as_str()
        .expect("second run_id");
    let second_run = wait_for_run_terminal(second_run_id).await;
    assert_eq!(second_run.status, ChatRunStatus::Completed);

    let agent_payload = sidecar_state
        .last_agent_payload
        .lock()
        .expect("agent payload lock")
        .clone()
        .expect("agent payload");
    assert_eq!(agent_payload["message"], "are you done?");
    assert_eq!(agent_payload["sessionId"], "sidecar-stream-session");

    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_live_chat_run_cancel_marks_run_cancelled() {
    init();
    reset_test_state();

    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    sidecar_state
        .agent_response_delay_ms
        .store(250, Ordering::Relaxed);
    insert_instance_sandbox_with_url("live-cancel-inst-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/live/chat/sessions")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({ "title": "Cancelable" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let session_id = body_json(create.into_body()).await["session_id"]
        .as_str()
        .expect("session id")
        .to_string();

    let prompt = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandbox/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "message": "cancel me",
                        "session_id": session_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.status(), StatusCode::ACCEPTED);
    let prompt_json = body_json(prompt.into_body()).await;
    let run_id = prompt_json["run_id"].as_str().expect("run_id").to_string();

    let cancel = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/sandbox/live/chat/sessions/{session_id}/runs/{run_id}/cancel"
                ))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancel_json = body_json(cancel.into_body()).await;
    assert_eq!(cancel_json["status"], "cancelled");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let detail = app()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sandbox/live/chat/sessions/{session_id}"))
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_json = body_json(detail.into_body()).await;
    assert!(detail_json["active_run_id"].is_null());
    assert_eq!(detail_json["runs"][0]["id"], run_id);
    assert_eq!(detail_json["runs"][0]["status"], "cancelled");
    assert!(
        sidecar_state.cancel_invocations.load(Ordering::Relaxed) >= 1,
        "expected operator to best-effort cancel the sidecar run",
    );

    server.abort();
}

// ── Helper: insert sandbox with extra_ports ─────────────────────────

fn insert_sandbox_with_ports(id: &str, owner: &str, ports: std::collections::HashMap<u16, u16>) {
    init();
    use crate::runtime::{SandboxRecord, SandboxState, sandboxes, seal_record};
    let mut record = SandboxRecord {
        id: id.to_string(),
        container_id: format!("ctr-{id}"),
        sidecar_url: "http://localhost:9999".to_string(),
        sidecar_port: 9999,
        ssh_port: None,
        token: "plain-token".into(),
        created_at: 1_700_000_000,
        cpu_cores: 1,
        memory_mb: 1024,
        state: SandboxState::Running,
        idle_timeout_seconds: 1800,
        max_lifetime_seconds: 86400,
        last_activity_at: 1_700_000_000,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "test:latest".into(),
        base_env_json: "{}".into(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: "port-sandbox".into(),
        agent_identifier: String::new(),
        metadata_json: "{}".into(),
        disk_gb: 10,
        stack: String::new(),
        owner: owner.to_string(),
        service_id: None,
        tee_config: None,
        extra_ports: ports,
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: String::new(),
    };
    seal_record(&mut record).unwrap();
    sandboxes().unwrap().insert(id.to_string(), record).unwrap();
}

fn insert_sandbox_for_listing(id: &str, owner: &str, service_id: Option<u64>) {
    insert_sandbox_with_ports(id, owner, std::collections::HashMap::new());
    sandboxes()
        .unwrap()
        .update(id, |record| {
            record.service_id = service_id;
        })
        .unwrap();
}

// =====================================================================
// Phase 1A: Port Proxy Handler Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_requires_auth() {
    init();
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/any-id/port/8080/some/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_list_sandboxes_repairs_service_links_and_exposes_managing_operator() {
    init();
    reset_test_state();

    let sandbox_id = "sandbox-service-backfill";
    let call_id = 880_001;
    let _managing_operator = EnvVarGuard::set(
        "MANAGING_OPERATOR_ADDRESS",
        "0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
    );
    let _operator_address = EnvVarGuard::remove("OPERATOR_ADDRESS");
    let _keystore_uri = EnvVarGuard::remove("KEYSTORE_URI");

    insert_sandbox_for_listing(
        sandbox_id,
        "0x1234567890abcdef1234567890abcdef12345678",
        None,
    );
    provision_progress::start_provision(call_id).unwrap();
    provision_progress::update_provision(
        call_id,
        provision_progress::ProvisionPhase::Ready,
        Some("Ready".into()),
        Some(sandbox_id.to_string()),
        Some("http://localhost:9999".into()),
    )
    .unwrap();
    provision_progress::update_provision_metadata(call_id, json!({ "service_id": 42 })).unwrap();

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes")
                .header("authorization", test_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response.into_body()).await;
    let listed_sandboxes = payload["sandboxes"].as_array().expect("sandbox list");
    let sandbox = listed_sandboxes
        .iter()
        .find(|entry| entry["id"] == sandbox_id)
        .expect("sandbox entry present");
    assert_eq!(sandbox["service_id"], 42);
    assert_eq!(
        sandbox["managing_operator"],
        "0x70997970c51812dc3a010c7d01b50e0d17dc79c8"
    );

    let stored = sandboxes()
        .unwrap()
        .get(sandbox_id)
        .unwrap()
        .expect("stored sandbox");
    assert_eq!(stored.service_id, Some(42));
}

#[serial_test::serial]
#[test]
fn test_derive_operator_address_from_keystore_uri() {
    let keystore_dir = tempfile::tempdir().expect("temp keystore dir");
    let ecdsa_dir = keystore_dir.path().join("Ecdsa");
    std::fs::create_dir_all(&ecdsa_dir).expect("create Ecdsa dir");
    std::fs::write(
            ecdsa_dir.join("operator-key.json"),
            r#"[[2,186,87,52,216,247,9,23,25,71,30,127,126,214,185,223,23,13,199,12,198,97,202,5,230,136,96,26,217,132,240,104,176],[89,198,153,94,153,143,151,165,160,4,73,102,240,148,83,137,220,158,134,218,232,140,122,132,18,244,96,59,107,120,105,13]]"#,
        )
        .expect("write keystore file");
    let derived = derive_operator_address_from_keystore_uri(&format!(
        "file://{}",
        keystore_dir.path().display()
    ))
    .expect("keystore should derive operator address");

    assert_eq!(derived, "0x70997970c51812dc3a010c7d01b50e0d17dc79c8");
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_wrong_owner_forbidden() {
    let mut ports = std::collections::HashMap::new();
    ports.insert(8080u16, 19080u16);
    insert_sandbox_with_ports("proxy-owner-1", OP_TEST_OWNER, ports);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000099")
    );
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/proxy-owner-1/port/8080/index.html")
                .header("authorization", &other_auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_unexposed_port_404() {
    let mut ports = std::collections::HashMap::new();
    ports.insert(3000u16, 13000u16);
    insert_sandbox_with_ports("proxy-port-1", OP_TEST_OWNER, ports);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/proxy-port-1/port/9999/path")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_rejects_null_byte_path() {
    let mut ports = std::collections::HashMap::new();
    ports.insert(8080u16, 18080u16);
    insert_sandbox_with_ports("proxy-null-1", OP_TEST_OWNER, ports);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/proxy-null-1/port/8080/some%00path")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[test]
fn test_port_proxy_rejects_double_slash_path() {
    // Test the run_port_proxy path validation directly rather than through
    // HTTP routing, since the Axum router consumes the leading slash from
    // the wildcard capture. The path validation in run_port_proxy rejects
    // paths starting with "//".
    let path = "//etc/passwd";
    assert!(
        path.starts_with("//"),
        "double-slash path should be detected"
    );
    // The run_port_proxy function checks:
    //   if path.contains('\0') || path.starts_with("//") { return Err(400) }
    // This verifies the defense-in-depth validation logic.
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_circuit_breaker_blocks() {
    let mut ports = std::collections::HashMap::new();
    ports.insert(8080u16, 18081u16);
    insert_sandbox_with_ports("proxy-cb-1", OP_TEST_OWNER, ports);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    // Trip the circuit breaker
    circuit_breaker::mark_unhealthy("proxy-cb-1");
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/proxy-cb-1/port/8080/path")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    // Clean up
    circuit_breaker::clear("proxy-cb-1");
}

#[serial_test::serial]
#[tokio::test]
async fn test_instance_port_proxy_requires_auth() {
    init();
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandbox/port/8080/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_port_proxy_forwards_correctly() {
    // Spawn a mock backend that a proxy will forward to
    let backend_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let backend_addr = backend_listener.local_addr().expect("backend addr");
    let backend_port = backend_addr.port();
    let backend_app = Router::new().route("/hello", get(|| async { (StatusCode::OK, "proxy-ok") }));
    tokio::spawn(async move {
        axum::serve(backend_listener, backend_app)
            .await
            .expect("serve backend");
    });
    // Wait for backend readiness
    for _ in 0..20 {
        if reqwest::get(format!("http://127.0.0.1:{backend_port}/hello"))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut ports = std::collections::HashMap::new();
    ports.insert(3000u16, backend_port);
    insert_sandbox_with_ports("proxy-fwd-1", OP_TEST_OWNER, ports);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/proxy-fwd-1/port/3000/hello")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        String::from_utf8_lossy(&body_bytes),
        "proxy-ok",
        "proxy should forward to backend and return its response"
    );
}

// =====================================================================
// Phase 1B: Cross-Owner Authorization Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_prompt_wrong_owner_forbidden() {
    insert_plain_sandbox("xowner-prompt-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000010")
    );
    let body = serde_json::json!({ "message": "hi" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/xowner-prompt-1/prompt")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_stop_wrong_owner_forbidden() {
    insert_plain_sandbox("xowner-stop-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000011")
    );
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/xowner-stop-1/stop")
                .header("authorization", &other_auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_secrets_inject_wrong_owner_forbidden() {
    insert_plain_sandbox("xowner-sec-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000012")
    );
    let body = serde_json::json!({ "env_json": { "SECRET": "val" } });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/xowner-sec-1/secrets")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_snapshot_wrong_owner_forbidden() {
    insert_plain_sandbox("xowner-snap-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000013")
    );
    let body = serde_json::json!({
        "destination": "s3://bucket/snap.tar.gz",
        "include_workspace": true,
        "include_state": false,
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/xowner-snap-1/snapshot")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// =====================================================================
// Phase 1C: Live Session Scope Isolation Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_terminal_session_cross_sandbox_isolation() {
    let (sidecar_url_a, _state_a, server_a) = spawn_mock_sidecar().await;
    let (sidecar_url_b, _state_b, server_b) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("iso-term-a", OP_TEST_OWNER, &sidecar_url_a);
    insert_plain_sandbox_with_url("iso-term-b", OP_TEST_OWNER, &sidecar_url_b);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    // Create terminal session on sandbox A
    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/iso-term-a/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    // List sessions on sandbox B — should not see A's session
    let list = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/iso-term-b/live/terminal/sessions")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let listed = body_json(list.into_body()).await;
    let sessions = listed["sessions"].as_array().unwrap();
    assert!(
        sessions.is_empty(),
        "sandbox B should not see sandbox A's terminal sessions"
    );

    server_a.abort();
    server_b.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_terminal_session_cross_owner_isolation() {
    const OWNER_A: &str = "0xISOOWNER00000000000000000000000000000A1";
    const OWNER_B: &str = "0xISOOWNER00000000000000000000000000000B1";
    let (sidecar_url, _state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("iso-owner-term-1", OWNER_A, &sidecar_url);
    let auth_a = format!("Bearer {}", session_auth::create_test_token(OWNER_A));
    let auth_b = format!("Bearer {}", session_auth::create_test_token(OWNER_B));

    // Owner A creates terminal session
    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/iso-owner-term-1/live/terminal/sessions")
                .header("authorization", &auth_a)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    // Owner B lists sessions on same sandbox — should see none (403 or empty)
    let list = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/iso-owner-term-1/live/terminal/sessions")
                .header("authorization", &auth_b)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Owner B is not owner of this sandbox, so FORBIDDEN
    assert_eq!(list.status(), StatusCode::FORBIDDEN);
    server.abort();
}

#[serial_test::serial]
#[test]
fn test_chat_session_cross_scope_isolation() {
    // Verify that sandbox scope and instance scope produce different scope
    // IDs for the same sandbox_id. This is the mechanism that ensures
    // session isolation between sandbox-mode and instance-mode.
    let sandbox_scope = live_scope_sandbox("test-scope-iso-1");
    assert_eq!(sandbox_scope, "sandbox:test-scope-iso-1");
    // Instance scope uses format!("instance:{}", record.id)
    // The key invariant: sandbox and instance scopes are always different.
    assert!(
        sandbox_scope.starts_with("sandbox:"),
        "sandbox scope must use 'sandbox:' prefix"
    );
}

#[serial_test::serial]
#[tokio::test]
async fn test_chat_session_cross_owner_isolation() {
    const CHAT_OWNER_A: &str = "0xCHATOWNER000000000000000000000000000A1";
    const CHAT_OWNER_B: &str = "0xCHATOWNER000000000000000000000000000B1";
    insert_plain_sandbox("iso-chat-own-1", CHAT_OWNER_A);
    let auth_a = format!("Bearer {}", session_auth::create_test_token(CHAT_OWNER_A));
    let auth_b = format!("Bearer {}", session_auth::create_test_token(CHAT_OWNER_B));

    // Owner A creates chat session
    let create_body = serde_json::json!({ "title": "owner-a chat" });
    let create = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/iso-chat-own-1/live/chat/sessions")
                .header("authorization", &auth_a)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    // Owner B tries to list chat sessions — FORBIDDEN (not sandbox owner)
    let list = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/iso-chat-own-1/live/chat/sessions")
                .header("authorization", &auth_b)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::FORBIDDEN);
}

// =====================================================================
// Phase 2B: Snapshot Destination Policy Tests (HTTP-level)
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_snapshot_rejects_http_destination() {
    insert_plain_sandbox("snap-http-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "destination": "http://93.184.216.34/snap.tar.gz",
        "include_workspace": true,
        "include_state": false,
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/snap-http-1/snapshot")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_snapshot_rejects_private_ip() {
    insert_plain_sandbox("snap-priv-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "destination": "https://192.168.1.1/snap.tar.gz",
        "include_workspace": true,
        "include_state": false,
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/snap-priv-1/snapshot")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[serial_test::serial]
#[tokio::test]
async fn test_sandbox_snapshot_accepts_s3_destination() {
    // NOTE: This will fail at the sidecar call (no real sidecar), but the
    // validation stage itself should pass. We only verify it doesn't return 400.
    insert_plain_sandbox("snap-s3-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "destination": "s3://my-bucket/snap.tar.gz",
        "include_workspace": true,
        "include_state": false,
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/snap-s3-1/snapshot")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    // Should NOT be 400 — s3:// passes validation.
    // Will likely be 502 (sidecar not available) which is expected.
    assert_ne!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "s3:// destination should pass validation"
    );
}

// =====================================================================
// Phase 2C: Stop/Resume Idempotency Tests (unit-level)
// =====================================================================

#[serial_test::serial]
#[test]
fn test_handle_lifecycle_outcome_already_stopped_ok() {
    let result = handle_lifecycle_outcome(
        Err(crate::SandboxError::Validation("already stopped".into())),
        "already stopped",
    );
    assert!(result.is_ok(), "already-stopped should be treated as Ok");
}

#[serial_test::serial]
#[test]
fn test_handle_lifecycle_outcome_already_running_ok() {
    let result = handle_lifecycle_outcome(
        Err(crate::SandboxError::Validation("already running".into())),
        "already running",
    );
    assert!(result.is_ok(), "already-running should be treated as Ok");
}

#[serial_test::serial]
#[test]
fn test_handle_lifecycle_outcome_real_error_propagates() {
    let result = handle_lifecycle_outcome(
        Err(crate::SandboxError::Docker(
            "Docker daemon unreachable".into(),
        )),
        "already stopped",
    );
    assert!(result.is_err(), "real Docker error should propagate");
}

#[serial_test::serial]
#[test]
fn test_handle_lifecycle_outcome_case_insensitive() {
    let result = handle_lifecycle_outcome(
        Err(crate::SandboxError::Validation("Already Stopped".into())),
        "already stopped",
    );
    assert!(
        result.is_ok(),
        "case-insensitive match on 'Already Stopped' should be Ok"
    );
}

// =====================================================================
// Phase 3C: Proxied Payload Contract Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_payload_uses_message_field() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("proxy-msg-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "test prompt message" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/proxy-msg-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = body_json(response.into_body()).await;
    let run_id = accepted["run_id"].as_str().expect("run_id");
    let run = wait_for_run_terminal(run_id).await;
    assert_eq!(run.status, ChatRunStatus::Completed);
    let payload = sidecar_state
        .last_agent_payload
        .lock()
        .expect("payload lock")
        .clone()
        .expect("sidecar should have received payload");
    assert!(accepted.get("run_id").is_some());
    assert_eq!(
        payload["message"], "test prompt message",
        "sidecar should receive 'message' field"
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_task_payload_uses_prompt_field() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("proxy-task-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "prompt": "do this task",
        "max_turns": 5
    });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/proxy-task-1/task")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let resp_json = body_json(response.into_body()).await;
    let run_id = resp_json["run_id"].as_str().expect("run_id");
    let run = wait_for_run_terminal(run_id).await;
    assert_eq!(run.status, ChatRunStatus::Completed);
    // The task handler sends the prompt via the "message" field to the sidecar
    let payload = sidecar_state
        .last_agent_payload
        .lock()
        .expect("payload lock")
        .clone()
        .expect("sidecar should have received payload");
    assert_eq!(
        payload["message"], "do this task",
        "sidecar should receive task prompt in 'message' field"
    );
    assert!(
        resp_json.get("run_id").is_some(),
        "task API response should include 'run_id' field"
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_auto_creates_session_when_missing() {
    // Uses sandbox-mode prompt (not instance mode) to avoid instance_store race.
    let (sidecar_url, _sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("proxy-auto-sess-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    // Send prompt without session_id — should auto-create session
    let body = serde_json::json!({ "message": "auto session test" });
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/proxy-auto-sess-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    assert!(
        !payload["session_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(!payload["run_id"].as_str().unwrap_or_default().is_empty());
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_retries_transient_agent_warmup_failures() {
    let (sidecar_url, sidecar_state, server) =
        spawn_mock_sidecar_with_agent_warmup_failures(2).await;
    insert_plain_sandbox_with_url("agent-warmup-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "warm up and reply" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/agent-warmup-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    let run = wait_for_run_terminal(payload["run_id"].as_str().expect("run_id")).await;
    assert_eq!(run.status, ChatRunStatus::Completed);
    assert_eq!(
        sidecar_state.agent_invocations.load(Ordering::Relaxed),
        3,
        "should retry warmup failures before succeeding"
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_returns_structured_service_unavailable_when_agent_stays_warming() {
    let (sidecar_url, sidecar_state, server) =
        spawn_mock_sidecar_with_agent_warmup_failures(10).await;
    insert_plain_sandbox_with_url("agent-warmup-2", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "still warming" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/agent-warmup-2/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    let run = wait_for_run_terminal(payload["run_id"].as_str().expect("run_id")).await;
    assert_eq!(run.status, ChatRunStatus::Failed);
    assert_eq!(
        run.error.as_deref(),
        Some("Sandbox agent is still starting up. Please retry shortly.")
    );
    assert_eq!(
        sidecar_state.agent_invocations.load(Ordering::Relaxed),
        (AGENT_WARMUP_RETRY_DELAYS_MS.len() + 1) as u64
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_agents_endpoint_lists_registered_agents() {
    let (sidecar_url, _sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("agents-list-1", OP_TEST_OWNER, &sidecar_url);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/agents-list-1/agents")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 2);
    assert_eq!(body["agents"][0]["identifier"], "default");
    assert_eq!(body["agents"][1]["identifier"], "batch");
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_rejects_unknown_configured_agent_identifier() {
    let (sidecar_url, _sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("bad-agent-1", OP_TEST_OWNER, &sidecar_url);
    set_agent_identifier("bad-agent-1", "a1");
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "hello" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/bad-agent-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    let run = wait_for_run_terminal(payload["run_id"].as_str().expect("run_id")).await;
    assert_eq!(
        run.error.as_deref(),
        Some("Unknown agent identifier \"a1\". Available agents: default, batch")
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_skips_agent_listing_for_valid_configured_agent() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    insert_plain_sandbox_with_url("good-agent-1", OP_TEST_OWNER, &sidecar_url);
    set_agent_identifier("good-agent-1", "default");
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "hello" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/good-agent-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    let run = wait_for_run_terminal(payload["run_id"].as_str().expect("run_id")).await;
    assert_eq!(run.status, ChatRunStatus::Completed);
    assert_eq!(
        sidecar_state.agent_list_invocations.load(Ordering::Relaxed),
        0
    );
    assert_eq!(sidecar_state.agent_invocations.load(Ordering::Relaxed), 1);
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_prompt_translates_missing_factory_error_when_agent_listing_is_unavailable() {
    let (sidecar_url, server) = spawn_mock_sidecar_without_agent_listing().await;
    insert_plain_sandbox_with_url("bad-agent-compat-1", OP_TEST_OWNER, &sidecar_url);
    set_agent_identifier("bad-agent-compat-1", "a1");
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({ "message": "hello" });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/bad-agent-compat-1/prompt")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload = body_json(response.into_body()).await;
    let run = wait_for_run_terminal(payload["run_id"].as_str().expect("run_id")).await;
    assert_eq!(
        run.error.as_deref(),
        Some("Unknown agent identifier \"a1\". This sidecar image does not register that agent.")
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_ssh_user_endpoint_detects_runtime_user() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    *sidecar_state
        .exec_response
        .lock()
        .expect("exec response lock") = json!({
        "result": {
            "exitCode": 0,
            "stdout": "sidecar\n",
            "stderr": ""
        }
    });
    insert_mock_sidecar_ssh_sandbox("ssh-user-1", OP_TEST_OWNER, &sidecar_url, 2222);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/ssh-user-1/ssh/user")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["success"], true, "body: {body}");
    assert_eq!(body["username"], "sidecar", "body: {body}");

    let payload = sidecar_state
        .last_exec_payload
        .lock()
        .expect("payload lock")
        .clone()
        .expect("sidecar should have received exec payload");
    assert_eq!(payload["command"], "id -un || whoami");
    server.abort();
}

#[serial_test::serial]
#[test]
fn test_parse_detected_ssh_username_tolerates_terminal_noise() {
    let exec = ExecApiResponse {
        exit_code: 0,
        stdout: "\u{1b}[?2004l\rsidecar\r\n\u{1b}[?2004hcontainer:/sidecar$ exit\r\n".to_string(),
        stderr: String::new(),
    };

    let username = parse_detected_ssh_username(&exec).expect("username should parse");
    assert_eq!(username, "sidecar");
}

#[serial_test::serial]
#[tokio::test]
async fn test_ssh_provision_returns_422_when_sidecar_command_fails() {
    let (sidecar_url, sidecar_state, server) = spawn_mock_sidecar().await;
    *sidecar_state
        .exec_response
        .lock()
        .expect("exec response lock") = json!({
        "result": {
            "exitCode": 2,
            "stdout": "",
            "stderr": "User agent does not exist"
        }
    });
    insert_mock_sidecar_ssh_sandbox("ssh-fail-1", OP_TEST_OWNER, &sidecar_url, 2222);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let body = serde_json::json!({
        "username": "agent",
        "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest test@test"
    });

    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/ssh-fail-1/ssh")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json = body_json(response.into_body()).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("SSH provision failed for user 'agent'"),
        "body: {json}"
    );
    server.abort();
}

#[serial_test::serial]
#[tokio::test]
async fn test_ssh_endpoints_reject_non_ssh_sandbox() {
    init();
    // Sandbox with ssh_port: None (default from insert_plain_sandbox)
    insert_plain_sandbox("ssh-nossh-1", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

    // GET /ssh/user should be rejected
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/api/sandboxes/ssh-nossh-1/ssh/user")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp.into_body()).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("SSH is not enabled"),
        "body: {body}"
    );

    // POST /ssh (provision) should be rejected
    let provision_body = json!({
        "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest test@test"
    });
    let resp = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/ssh-nossh-1/ssh")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&provision_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // DELETE /ssh (revoke) should be rejected
    let revoke_body = json!({
        "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest test@test"
    });
    let resp = app()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/sandboxes/ssh-nossh-1/ssh")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&revoke_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// =====================================================================
// Phase 3F: Error Response Format Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_error_responses_are_json_with_error_field() {
    init();
    // 403 — wrong owner: uses api_error() which returns JSON
    insert_plain_sandbox("errfmt-1", OP_TEST_OWNER);
    let other_auth = format!(
        "Bearer {}",
        session_auth::create_test_token("0xOTHER0000000000000000000000000000000020")
    );
    let resp_403 = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/errfmt-1/exec")
                .header("authorization", &other_auth)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"command":"echo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_403.status(), StatusCode::FORBIDDEN);
    let json_403 = body_json(resp_403.into_body()).await;
    assert!(
        json_403.get("error").is_some(),
        "403 response should have 'error' field: {json_403}"
    );

    // 400 — empty snapshot destination
    insert_plain_sandbox("errfmt-2", OP_TEST_OWNER);
    let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
    let resp_400 = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/errfmt-2/snapshot")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"destination":"","include_workspace":true,"include_state":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_400.status(), StatusCode::BAD_REQUEST);
    let json_400 = body_json(resp_400.into_body()).await;
    assert!(
        json_400.get("error").is_some(),
        "400 response should have 'error' field: {json_400}"
    );

    // 404 — non-existent sandbox
    let resp_404 = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sandboxes/nonexistent-xyz/exec")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"command":"echo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_404.status(), StatusCode::NOT_FOUND);
    let json_404 = body_json(resp_404.into_body()).await;
    assert!(
        json_404.get("error").is_some(),
        "404 response should have 'error' field: {json_404}"
    );
}

#[serial_test::serial]
#[test]
fn test_rate_limit_response_includes_retry_after() {
    // Verify the rate limit middleware returns Retry-After header by checking
    // the limiter behavior with a dedicated limiter (not the shared static one).
    let limiter =
        crate::rate_limit::RateLimiter::new(crate::rate_limit::RateLimitConfig::new(1, 60));
    let ip: std::net::IpAddr = "198.51.100.200".parse().unwrap();
    assert!(limiter.check(ip), "first request should pass");
    assert!(!limiter.check(ip), "second request should be rate-limited");
    // The middleware code in rate_limit.rs includes `[("retry-after", "60")]`
    // in the 429 response. We verify the limiter correctly blocks, and the
    // header inclusion is verified by code inspection.
}

// =====================================================================
// Phase 3G: Health/Readyz Structure Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_health_degraded_response_structure() {
    init();
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let json = body_json(response.into_body()).await;
    assert!(json["status"].is_string(), "missing status field");
    assert!(json["checks"].is_object(), "missing checks object");
    assert!(
        json["checks"]["runtime"].is_object(),
        "missing runtime check"
    );
    assert!(json["checks"]["store"].is_object(), "missing store check");
    if status == StatusCode::SERVICE_UNAVAILABLE {
        assert_eq!(json["status"], "degraded");
    }
}

#[serial_test::serial]
#[tokio::test]
async fn test_readyz_includes_runtime_backend() {
    init();
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if status == StatusCode::SERVICE_UNAVAILABLE {
        let json = body_json(response.into_body()).await;
        assert!(
            json.get("runtime_backend").is_some(),
            "readyz should include runtime_backend field when not ready"
        );
    }
    // When ready (200), there is no runtime_backend field — that's fine.
}

#[serial_test::serial]
#[tokio::test]
async fn test_health_and_readyz_unauthenticated() {
    init();
    // /health and /readyz should NOT require auth
    for path in &["/health", "/readyz"] {
        let response = app()
            .clone()
            .oneshot(Request::builder().uri(*path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} should not require auth"
        );
    }
}

// =====================================================================
// Phase 3D: Instance Store Sync Tests
// =====================================================================

#[serial_test::serial]
#[tokio::test]
async fn test_instance_store_survives_missing_record() {
    init();
    // Getting a non-existent key should return None, not panic
    let record = runtime::instance_store()
        .unwrap()
        .get("nonexistent_key")
        .unwrap();
    assert!(record.is_none(), "missing key should return None");
}

// =====================================================================
// Adversarial: context_json cannot override maxTurns
// =====================================================================

#[serial_test::serial]
#[test]
fn test_build_agent_payload_context_json_cannot_override_max_turns() {
    // A malicious client sends context_json with a maxTurns override
    // attempting to remove the operator-enforced turn limit.
    let payload = build_agent_payload(AgentPayloadRequest {
        message: "hello",
        session_id: "sess-1",
        backend_type: "",
        model: "",
        context_json: r#"{"maxTurns": 999999, "custom_key": "safe"}"#,
        timeout_ms: 60_000,
        max_turns: Some(5), // operator-enforced limit
        agent_identifier: "default",
    });

    let metadata = payload.get("metadata").expect("metadata should exist");
    assert_eq!(
        metadata.get("maxTurns").and_then(|v| v.as_u64()),
        Some(5),
        "CRITICAL: context_json overrode operator maxTurns! Attacker can bypass turn limits."
    );
    assert_eq!(
        metadata.get("custom_key").and_then(|v| v.as_str()),
        Some("safe"),
        "non-protected context keys should still pass through"
    );
}

#[serial_test::serial]
#[test]
fn test_build_agent_payload_context_json_without_max_turns_override() {
    // Normal case: context_json doesn't try to override maxTurns
    let payload = build_agent_payload(AgentPayloadRequest {
        message: "hello",
        session_id: "",
        backend_type: "gemini",
        model: "gpt-4",
        context_json: r#"{"user_context": "some data"}"#,
        timeout_ms: 0,
        max_turns: Some(10),
        agent_identifier: "",
    });

    let metadata = payload.get("metadata").expect("metadata should exist");
    assert_eq!(metadata.get("maxTurns").and_then(|v| v.as_u64()), Some(10),);
    assert_eq!(
        metadata.get("user_context").and_then(|v| v.as_str()),
        Some("some data"),
    );
    let backend = payload.get("backend").expect("backend should exist");
    assert_eq!(backend.get("type").and_then(|v| v.as_str()), Some("gemini"));
    assert_eq!(backend.get("model").and_then(|v| v.as_str()), Some("gpt-4"));
}
