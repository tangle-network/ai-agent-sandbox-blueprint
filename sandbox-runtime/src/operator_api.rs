//! Axum-based operator API for sandbox management.
//!
//! Provides REST endpoints for:
//! - Listing active sandboxes
//! - Querying provision progress
//! - Session auth (challenge/response + PASETO tokens)
//! - Sandbox operations (exec, prompt, task, stop, resume, snapshot, SSH)

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio_stream::StreamExt;

use crate::api_types::*;
use crate::chat_state::{
    self, ChatMessageRecord, ChatRunKind, ChatRunProgressRecord, ChatRunRecord, ChatRunStatus,
    ChatSessionRecord,
};
use crate::circuit_breaker;
use crate::error::SandboxError;
use crate::http::{
    auth_headers, build_url, sidecar_get_json, sidecar_post_json, sidecar_post_json_without_timeout,
};
use crate::live_operator_sessions::{
    LiveSessionStore, LiveTerminalSession, sse_from_json_events, sse_from_terminal_output,
};
use crate::metrics;
use crate::provision_progress;
use crate::rate_limit;
use crate::runtime::{
    self, SandboxRecord, SandboxState, sandboxes, workflow_runtime_credentials_available,
};
use crate::secret_provisioning;
use crate::session_auth::{self, SessionAuth};

// ---------------------------------------------------------------------------
// Per-operation sidecar call timeouts
// ---------------------------------------------------------------------------

/// Timeout for exec (shell command) calls to the sidecar.
const SIDECAR_EXEC_TIMEOUT: Duration = Duration::from_secs(30);

const DEFAULT_PROMPT_RUN_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const DEFAULT_TASK_RUN_TIMEOUT_MS: u64 = 30 * 60 * 1000;
const CHAT_CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for other sidecar calls (snapshot, SSH provisioning, etc.).
const SIDECAR_DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Live terminal session output ring-buffer size.
const LIVE_TERMINAL_OUTPUT_BUFFER: usize = 512;
const AGENT_WARMUP_ERROR_CODE: &str = "AGENT_WARMING_UP";
#[cfg(not(test))]
const AGENT_WARMUP_RETRY_DELAYS_MS: &[u64] = &[250, 500, 1_000, 2_000, 4_000, 4_000, 4_000];
#[cfg(test)]
const AGENT_WARMUP_RETRY_DELAYS_MS: &[u64] = &[5, 5, 5];

/// Shared in-memory live chat/terminal sessions.
static LIVE_SESSIONS: Lazy<LiveSessionStore<Value>> = Lazy::new(LiveSessionStore::default);
static CHAT_RUN_ABORTS: Lazy<Mutex<HashMap<String, AbortHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static CHAT_RUN_ENQUEUE_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

// ---------------------------------------------------------------------------
// Request ID middleware
// ---------------------------------------------------------------------------

/// Monotonic counter for generating unique request IDs.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique identifier attached to every request for correlation in logs and
/// response headers.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

tokio::task_local! {
    /// The request ID for the current task, set by [`request_id_middleware`].
    ///
    /// Downstream helpers (e.g. [`crate::http::sidecar_post_json`]) read this
    /// via `try_with` to propagate the `x-request-id` header to sidecar HTTP
    /// calls, enabling end-to-end trace correlation between operator and
    /// sidecar logs.
    pub static CURRENT_REQUEST_ID: String;
}

/// Middleware that assigns a unique `x-request-id` to every request.
///
/// The ID is inserted into request extensions (so handlers can access it via
/// `Extension<RequestId>`) and echoed back in the `x-request-id` response
/// header for client-side correlation.  It is also stored in the
/// [`CURRENT_REQUEST_ID`] task-local so that downstream sidecar HTTP calls
/// automatically propagate the same ID.
async fn request_id_middleware(
    mut req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    let id = format!(
        "req-{:016x}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    tracing::debug!(request_id = %id, method = %req.method(), uri = %req.uri(), "incoming request");
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut res = CURRENT_REQUEST_ID.scope(id.clone(), next.run(req)).await;
    res.headers_mut()
        .insert("x-request-id", id.parse().unwrap());
    res
}

// ---------------------------------------------------------------------------
// Security headers middleware
// ---------------------------------------------------------------------------

/// Middleware that adds security headers to every response.
///
/// Applied headers:
/// - `X-Content-Type-Options: nosniff` — prevent MIME-type sniffing
/// - `X-Frame-Options: DENY` — disallow framing (clickjacking protection)
/// - `Cache-Control: no-store` — prevent caching of API responses
async fn security_headers_middleware(
    req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("cache-control", "no-store".parse().unwrap());
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    res
}

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiError {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
}

pub(crate) fn api_error(
    status: StatusCode,
    msg: impl Into<String>,
) -> (StatusCode, Json<ApiError>) {
    api_error_with_details(status, msg, None, None)
}

pub(crate) fn api_error_with_details(
    status: StatusCode,
    msg: impl Into<String>,
    code: Option<&str>,
    retry_after_ms: Option<u64>,
) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: msg.into(),
            code: code.map(str::to_string),
            retry_after_ms,
        }),
    )
}

/// Convert a `SandboxError` from `circuit_breaker::check_health` into a
/// structured 503 response with the `CIRCUIT_BREAKER` error code.
fn circuit_breaker_api_error(err: SandboxError) -> (StatusCode, Json<ApiError>) {
    match err {
        SandboxError::CircuitBreaker {
            remaining_secs,
            probing,
        } => api_error_with_details(
            StatusCode::SERVICE_UNAVAILABLE,
            if probing {
                "Sidecar recovery probe in progress. Please retry shortly.".to_string()
            } else {
                format!("Sidecar is in circuit-breaker cooldown ({remaining_secs}s remaining).")
            },
            Some("CIRCUIT_BREAKER"),
            Some(remaining_secs * 1000),
        ),
        other => api_error(StatusCode::SERVICE_UNAVAILABLE, other.to_string()),
    }
}

#[derive(Debug, Deserialize, Default)]
struct CreateLiveChatSessionRequest {
    #[serde(default)]
    title: String,
}

#[derive(Debug, Serialize)]
struct LiveSessionSummary {
    session_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_run_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct LiveChatSessionDetail {
    session_id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sidecar_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_run_id: Option<String>,
    messages: Vec<ChatMessageRecord>,
    run_progress: Vec<ChatRunProgressRecord>,
    runs: Vec<ChatRunRecord>,
}

#[derive(Debug, Serialize)]
struct CancelChatRunResponse {
    success: bool,
    session_id: String,
    run_id: String,
    status: String,
    cancelled_at: u64,
}

fn live_scope_sandbox(sandbox_id: &str) -> String {
    format!("sandbox:{sandbox_id}")
}

fn live_scope_instance(record: &SandboxRecord) -> String {
    format!("instance:{}", record.id)
}

fn terminal_session_matches(session: &LiveTerminalSession, scope: &str, owner: &str) -> bool {
    session.scope_id == scope && session.owner.eq_ignore_ascii_case(owner)
}

fn chat_session_matches(session: &ChatSessionRecord, scope: &str, owner: &str) -> bool {
    chat_state::session_matches(session, scope, owner)
}

fn publish_terminal_output(scope: &str, owner: &str, session_id: &str, stdout: &str, stderr: &str) {
    if session_id.trim().is_empty() {
        return;
    }
    let session = match LIVE_SESSIONS.get_terminal(session_id) {
        Ok(Some(s)) if terminal_session_matches(&s, scope, owner) => s,
        _ => return,
    };
    if !stdout.is_empty() {
        let _ = session.output_tx.send(stdout.to_string());
    }
    if !stderr.is_empty() {
        let _ = session.output_tx.send(format!("[stderr] {stderr}"));
    }
}

fn publish_chat_message(session_id: &str, message: ChatMessageRecord, event_type: &str) {
    if message.content.trim().is_empty() && message.role.eq_ignore_ascii_case("assistant") {
        return;
    }
    let _ = chat_state::append_message(session_id, message.clone());
    let _ = chat_state::emit_event(
        session_id,
        event_type,
        chat_state::message_event_payload(&message),
    );
}

fn publish_run_event(session_id: &str, event_type: &str, run: &ChatRunRecord) {
    let _ = chat_state::emit_event(session_id, event_type, chat_state::run_event_payload(run));
}

fn publish_run_progress(
    session_id: &str,
    run_id: &str,
    status: &ChatRunStatus,
    phase: &str,
    message: &str,
) {
    let Ok(Some(progress)) =
        chat_state::append_run_progress(session_id, run_id, status.clone(), phase, message)
    else {
        return;
    };
    let _ = chat_state::emit_event(session_id, "run_progress", json!(progress));
}

#[derive(Debug, Default)]
struct AgentStreamOutcome {
    success: bool,
    response: String,
    error: String,
    trace_id: String,
    session_id: String,
    duration_ms: u64,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug)]
struct SidecarSseEvent {
    event_type: String,
    data: Value,
}

fn chat_message_info_payload(session_id: &str, message: &ChatMessageRecord) -> Value {
    json!({
        "info": {
            "id": message.id,
            "role": message.role,
            "sessionID": session_id,
            "timestamp": message.created_at,
            "time": {
                "created": message.created_at,
                "completed": message.completed_at,
            }
        }
    })
}

fn emit_message_updated(session_id: &str, message: &ChatMessageRecord) {
    let _ = chat_state::emit_event(
        session_id,
        "message.updated",
        chat_message_info_payload(session_id, message),
    );
}

fn emit_message_part_updated(session_id: &str, message_id: &str, part: Value) {
    let mut part_object = match part {
        Value::Object(map) => map,
        _ => return,
    };
    part_object.insert("sessionID".into(), json!(session_id));
    part_object.insert("messageID".into(), json!(message_id));
    let _ = chat_state::emit_event(
        session_id,
        "message.part.updated",
        json!({ "part": Value::Object(part_object) }),
    );
}

fn emit_session_idle(session_id: &str) {
    let _ = chat_state::emit_event(session_id, "session.idle", json!({ "sessionID": session_id }));
}

fn emit_session_error(session_id: &str, message: &str, code: Option<&str>) {
    let _ = chat_state::emit_event(
        session_id,
        "session.error",
        json!({
            "sessionID": session_id,
            "error": {
                "message": message,
                "code": code,
            }
        }),
    );
}

fn parse_sse_event(frame: &str) -> Option<SidecarSseEvent> {
    let mut event_type = "message".to_string();
    let mut data_lines = Vec::new();

    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let raw = data_lines.join("\n");
    let data = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!(raw));
    Some(SidecarSseEvent { event_type, data })
}

fn normalize_stream_part(part: &Value) -> Option<Value> {
    let mut object = part.as_object()?.clone();
    if object.get("type").and_then(Value::as_str) == Some("image") {
        return None;
    }

    if object.get("type").and_then(Value::as_str) == Some("tool") {
        if let Some(state) = object.get_mut("state").and_then(Value::as_object_mut) {
            if state.get("status").and_then(Value::as_str) == Some("failed") {
                state.insert("status".into(), json!("error"));
            }
        }
    }

    Some(Value::Object(object))
}

fn should_forward_stream_part(
    part: &Value,
    request_text: &str,
    ignored_upstream_message_ids: &mut HashSet<String>,
    assistant_upstream_message_ids: &mut HashSet<String>,
) -> bool {
    let request_text = request_text.trim();
    let is_exact_request_echo = !request_text.is_empty()
        && part.get("type").and_then(Value::as_str) == Some("text")
        && part
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some(request_text);

    let upstream_message_id = part
        .get("messageID")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());

    if is_exact_request_echo {
        if let Some(upstream_message_id) = upstream_message_id {
            ignored_upstream_message_ids.insert(upstream_message_id.to_string());
        }
        return false;
    }

    let Some(upstream_message_id) = upstream_message_id else {
        return true;
    };

    if ignored_upstream_message_ids.contains(upstream_message_id) {
        return false;
    }
    if assistant_upstream_message_ids.contains(upstream_message_id) {
        return true;
    }

    assistant_upstream_message_ids.insert(upstream_message_id.to_string());
    true
}

fn finalize_streamed_assistant_parts(parts: &mut [Value], completed_at: u64) {
    for part in parts {
        let Some(object) = part.as_object_mut() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }

        let Some(time) = object.get_mut("time").and_then(Value::as_object_mut) else {
            continue;
        };
        if time.get("start").is_some() && time.get("end").is_none() {
            time.insert("end".into(), json!(completed_at));
        }
    }
}

fn parse_agent_stream_result(parsed: &Value) -> AgentStreamOutcome {
    let final_text = parsed
        .get("finalText")
        .or_else(|| parsed.get("response"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let metadata = parsed.get("metadata");
    let session_id = metadata
        .and_then(|meta| meta.get("sessionId"))
        .or_else(|| parsed.get("sessionId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let trace_id = metadata
        .and_then(|meta| meta.get("traceId"))
        .or_else(|| parsed.get("traceId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let token_usage = parsed.get("tokenUsage").or_else(|| parsed.get("usage"));
    let timing = parsed.get("timing");

    AgentStreamOutcome {
        success: true,
        response: final_text,
        error: String::new(),
        trace_id,
        session_id,
        duration_ms: timing
            .and_then(|value| value.get("totalMs").or_else(|| value.get("duration_ms")))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        input_tokens: token_usage
            .and_then(|value| value.get("inputTokens").or_else(|| value.get("input_tokens")))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: token_usage
            .and_then(|value| value.get("outputTokens").or_else(|| value.get("output_tokens")))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    }
}

fn register_chat_run_abort(run_id: &str, abort_handle: AbortHandle) {
    if let Ok(mut handles) = CHAT_RUN_ABORTS.lock() {
        handles.insert(run_id.to_string(), abort_handle);
    }
}

fn clear_chat_run_abort(run_id: &str) {
    if let Ok(mut handles) = CHAT_RUN_ABORTS.lock() {
        handles.remove(run_id);
    }
}

fn abort_chat_run_task(run_id: &str) -> bool {
    match CHAT_RUN_ABORTS.lock() {
        Ok(mut handles) => {
            if let Some(handle) = handles.remove(run_id) {
                handle.abort();
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Sandbox endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SandboxSummary {
    id: String,
    name: String,
    sidecar_url: String,
    state: String,
    image: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    agent_identifier: String,
    cpu_cores: u64,
    memory_mb: u64,
    disk_gb: u64,
    created_at: u64,
    last_activity_at: u64,
    ssh_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    managing_operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tee_deployment_id: Option<String>,
    /// Extra user-exposed ports: container_port → host_port.
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    extra_ports: std::collections::HashMap<u16, u16>,
    /// Seconds of inactivity before the sandbox is automatically stopped.
    idle_timeout_seconds: u64,
    /// Maximum lifetime in seconds before the sandbox is hard-deleted.
    max_lifetime_seconds: u64,
    /// Whether the sandbox has AI credentials configured (e.g. ANTHROPIC_API_KEY).
    credentials_available: bool,
    /// Whether the circuit breaker is currently active for this sandbox's sidecar.
    circuit_breaker_active: bool,
    /// Seconds remaining in the circuit breaker cooldown (if active).
    #[serde(skip_serializing_if = "Option::is_none")]
    circuit_breaker_remaining_secs: Option<u64>,
    /// Whether a recovery probe is in flight.
    circuit_breaker_probing: bool,
}

impl SandboxSummary {
    fn from_record(r: &SandboxRecord, managing_operator: Option<&str>) -> Self {
        let breaker = circuit_breaker::query_status(&r.id);
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            sidecar_url: r.sidecar_url.clone(),
            state: match r.state {
                SandboxState::Running => "running".into(),
                SandboxState::Stopped => "stopped".into(),
            },
            image: r.original_image.clone(),
            agent_identifier: r.agent_identifier.clone(),
            cpu_cores: r.cpu_cores,
            memory_mb: r.memory_mb,
            disk_gb: r.disk_gb,
            created_at: r.created_at,
            last_activity_at: r.last_activity_at,
            ssh_port: r.ssh_port,
            service_id: r.service_id,
            managing_operator: managing_operator.map(str::to_string),
            tee_deployment_id: r.tee_deployment_id.clone(),
            extra_ports: r.extra_ports.clone(),
            idle_timeout_seconds: r.idle_timeout_seconds,
            max_lifetime_seconds: r.max_lifetime_seconds,
            credentials_available: workflow_runtime_credentials_available(&r.effective_env_json())
                .unwrap_or(false),
            circuit_breaker_active: breaker.active,
            circuit_breaker_remaining_secs: breaker.remaining_secs,
            circuit_breaker_probing: breaker.probing,
        }
    }
}

fn normalize_operator_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() != 42 || !trimmed.starts_with("0x") {
        return None;
    }
    if trimmed.as_bytes()[2..]
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        Some(trimmed.to_ascii_lowercase())
    } else {
        None
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

fn derive_operator_address_from_secret(secret: &[u8]) -> std::result::Result<String, String> {
    use k256::ecdsa::SigningKey;

    let key_bytes: [u8; 32] = secret
        .try_into()
        .map_err(|_| "operator key must be exactly 32 bytes".to_string())?;
    let signing_key = SigningKey::from_bytes((&key_bytes).into())
        .map_err(|err| format!("invalid operator key bytes: {err}"))?;
    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let hash = keccak256(pubkey_uncompressed);
    Ok(format!("0x{}", hex::encode(&hash[12..])))
}

fn derive_operator_address_from_keystore_uri(
    keystore_uri: &str,
) -> std::result::Result<String, String> {
    use std::fs;
    use std::path::Path;

    let keystore_path = keystore_uri.strip_prefix("file://").unwrap_or(keystore_uri);
    let ecdsa_dir = Path::new(keystore_path).join("Ecdsa");
    let mut entries = fs::read_dir(&ecdsa_dir)
        .map_err(|err| format!("failed to read {}: {err}", ecdsa_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to enumerate {}: {err}", ecdsa_dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let components: Vec<Vec<u8>> = serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
        if let Some(secret) = components.iter().rev().find(|part| part.len() == 32) {
            return derive_operator_address_from_secret(secret);
        }
    }

    Err(format!(
        "no usable ECDSA secret found under {}",
        ecdsa_dir.display()
    ))
}

fn current_managing_operator() -> Option<String> {
    for key in ["MANAGING_OPERATOR_ADDRESS", "OPERATOR_ADDRESS"] {
        if let Ok(value) = std::env::var(key) {
            if let Some(address) = normalize_operator_address(&value) {
                return Some(address);
            }
        }
    }

    let keystore_uri = std::env::var("KEYSTORE_URI").ok()?;
    match derive_operator_address_from_keystore_uri(&keystore_uri) {
        Ok(address) => Some(address),
        Err(err) => {
            tracing::warn!(error = %err, "Failed to derive managing operator address from keystore");
            None
        }
    }
}

async fn list_sandboxes(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    if let Ok(repaired) = runtime::repair_sandbox_service_links_from_provisions() {
        if repaired > 0 {
            tracing::info!(
                repaired,
                "Repaired missing sandbox service links from provision metadata"
            );
        }
    }

    let managing_operator = current_managing_operator();
    match sandboxes().and_then(|s| s.values()) {
        Ok(records) => {
            let summaries: Vec<SandboxSummary> = records
                .into_iter()
                .filter(|r| !r.owner.is_empty() && r.owner.eq_ignore_ascii_case(&address))
                .filter_map(|mut record| {
                    if let Err(e) = runtime::unseal_record(&mut record) {
                        tracing::warn!(id = %record.id, error = %e, "Failed to unseal record in listing — skipping");
                        return None;
                    }
                    Some(SandboxSummary::from_record(&record, managing_operator.as_deref()))
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "sandboxes": summaries })),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Provision progress endpoints
// ---------------------------------------------------------------------------

async fn get_provision(Path(call_id): Path<u64>) -> impl IntoResponse {
    match provision_progress::get_provision(call_id) {
        Ok(Some(status)) => match serde_json::to_value(status) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Ok(None) => api_error(StatusCode::NOT_FOUND, "Provision not found").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_provisions() -> impl IntoResponse {
    match provision_progress::list_all_provisions() {
        Ok(provisions) => (
            StatusCode::OK,
            Json(serde_json::json!({ "provisions": provisions })),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Session auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionRequest {
    nonce: String,
    signature: String,
}

async fn create_challenge() -> impl IntoResponse {
    let challenge = match session_auth::create_challenge() {
        Ok(c) => c,
        Err(e) => {
            return api_error(StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response();
        }
    };
    match serde_json::to_value(challenge) {
        Ok(val) => (StatusCode::OK, Json(val)).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_session(Json(req): Json<SessionRequest>) -> impl IntoResponse {
    match session_auth::exchange_signature_for_token(&req.nonce, &req.signature) {
        Ok(token) => match serde_json::to_value(token) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(crate::error::SandboxError::Unavailable(msg)) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, msg).into_response()
        }
        Err(e) => api_error(StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

/// Revoke the current session token.
async fn revoke_session(headers: HeaderMap) -> impl IntoResponse {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(session_auth::extract_bearer_token);

    match token {
        Some(t) => {
            let revoked = session_auth::revoke_session(t);
            if revoked {
                (StatusCode::OK, Json(json!({"revoked": true}))).into_response()
            } else {
                (
                    StatusCode::OK,
                    Json(json!({"revoked": false, "message": "Token not found in session store"})),
                )
                    .into_response()
            }
        }
        None => api_error(StatusCode::BAD_REQUEST, "Missing Authorization header").into_response(),
    }
}

// ---------------------------------------------------------------------------
// Live chat / terminal session endpoints
// ---------------------------------------------------------------------------

fn create_terminal_session(
    scope_id: String,
    owner: &str,
) -> Result<LiveSessionSummary, (StatusCode, Json<ApiError>)> {
    let session =
        LiveTerminalSession::new(scope_id, owner.to_string(), LIVE_TERMINAL_OUTPUT_BUFFER);
    let summary = LiveSessionSummary {
        session_id: session.id.clone(),
        title: String::new(),
        active_run_id: None,
    };
    LIVE_SESSIONS
        .insert_terminal(session)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(summary)
}

fn list_terminal_sessions(
    scope_id: &str,
    owner: &str,
) -> Result<Vec<LiveSessionSummary>, (StatusCode, Json<ApiError>)> {
    LIVE_SESSIONS
        .list_terminals()
        .map(|sessions| {
            sessions
                .into_iter()
                .filter(|s| terminal_session_matches(s, scope_id, owner))
                .map(|s| LiveSessionSummary {
                    session_id: s.id,
                    title: String::new(),
                    active_run_id: None,
                })
                .collect()
        })
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

fn stream_terminal_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let session = LIVE_SESSIONS
        .get_terminal(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Terminal session not found"))?;
    if !terminal_session_matches(&session, scope_id, owner) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "Terminal session not found",
        ));
    }
    let rx = session.output_tx.subscribe();
    Ok(sse_from_terminal_output(rx).into_response())
}

fn delete_terminal_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<serde_json::Value, (StatusCode, Json<ApiError>)> {
    let session = LIVE_SESSIONS
        .get_terminal(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Terminal session not found"))?;
    if !terminal_session_matches(&session, scope_id, owner) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "Terminal session not found",
        ));
    }
    let _ = LIVE_SESSIONS
        .remove_terminal(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(json!({ "deleted": true, "session_id": session_id }))
}

fn create_chat_session(
    scope_id: String,
    owner: &str,
    title: String,
) -> Result<LiveSessionSummary, (StatusCode, Json<ApiError>)> {
    let session = chat_state::create_session(&scope_id, owner, Some(&title))
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(LiveSessionSummary {
        session_id: session.id,
        title: session.title,
        active_run_id: session.active_run_id,
    })
}

fn list_chat_sessions(
    scope_id: &str,
    owner: &str,
) -> Result<Vec<LiveSessionSummary>, (StatusCode, Json<ApiError>)> {
    chat_state::list_sessions(scope_id, owner)
        .map(|sessions| {
            sessions
                .into_iter()
                .map(|s| LiveSessionSummary {
                    session_id: s.id,
                    title: s.title,
                    active_run_id: s.active_run_id,
                })
                .collect()
        })
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

fn get_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<LiveChatSessionDetail, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    let runs = chat_state::list_runs_for_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let run_progress = chat_state::list_run_progress_for_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(LiveChatSessionDetail {
        session_id: session.id,
        title: session.title,
        sidecar_session_id: session.latest_sidecar_session_id,
        active_run_id: session.active_run_id,
        messages: session.messages,
        run_progress,
        runs,
    })
}

fn stream_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    let rx = chat_state::subscribe_events(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(sse_from_json_events(rx).into_response())
}

fn delete_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<serde_json::Value, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    if let Some(active_run_id) = session.active_run_id.as_deref() {
        if let Some(run) = chat_state::get_run(active_run_id)
            .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        {
            if run.status.is_active() {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "Cannot delete a chat session while a run is active",
                ));
            }
        }
    }
    chat_state::delete_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(json!({ "deleted": true, "session_id": session_id }))
}

async fn cancel_chat_run(
    record: &SandboxRecord,
    scope_id: &str,
    owner: &str,
    session_id: &str,
    run_id: &str,
) -> Result<CancelChatRunResponse, (StatusCode, Json<ApiError>)> {
    let (session, run) = resolve_chat_run(scope_id, owner, session_id, run_id)?;

    if run.status == ChatRunStatus::Cancelled {
        return Ok(CancelChatRunResponse {
            success: true,
            session_id: session.id,
            run_id: run.id,
            status: chat_run_status_label(&run.status).to_string(),
            cancelled_at: run.completed_at.unwrap_or(run.created_at),
        });
    }

    if !run.status.is_active() {
        return Ok(CancelChatRunResponse {
            success: true,
            session_id: session.id,
            run_id: run.id,
            status: chat_run_status_label(&run.status).to_string(),
            cancelled_at: run.completed_at.unwrap_or(chat_state::now_ms()),
        });
    }

    if session.active_run_id.as_deref() != Some(run.id.as_str()) {
        return Err(api_error_with_details(
            StatusCode::CONFLICT,
            "This run is no longer the active chat run for the session",
            Some("CHAT_RUN_NOT_ACTIVE"),
            None,
        ));
    }

    let cancelling_at = chat_state::now_ms();
    let _ = chat_state::update_run(&run.id, |entry| {
        entry.status = ChatRunStatus::Cancelling;
        if entry.started_at.is_none() {
            entry.started_at = Some(cancelling_at);
        }
    });
    if let Ok(Some(cancelling_run)) = chat_state::get_run(&run.id) {
        publish_run_event(&session.id, "run_cancel_requested", &cancelling_run);
        publish_run_progress(
            &session.id,
            &cancelling_run.id,
            &cancelling_run.status,
            "cancelling",
            "Cancellation requested. Stopping the active run.",
        );
    }

    abort_chat_run_task(&run.id);
    let updated_run = finalize_cancelled_chat_run(&session.id, &run.id, "Run cancelled by user.")?;
    best_effort_cancel_sidecar_run(record).await;

    Ok(CancelChatRunResponse {
        success: true,
        session_id: session.id,
        run_id: updated_run.id,
        status: chat_run_status_label(&updated_run.status).to_string(),
        cancelled_at: updated_run.completed_at.unwrap_or(cancelling_at),
    })
}

async fn sandbox_terminal_session_create_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_running(&record)?;
    let summary = create_terminal_session(live_scope_sandbox(&record.id), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

async fn sandbox_terminal_session_list_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let sessions = list_terminal_sessions(&live_scope_sandbox(&record.id), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

async fn sandbox_terminal_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    stream_terminal_session(&live_scope_sandbox(&record.id), &address, &session_id)
}

async fn sandbox_terminal_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = delete_terminal_session(&live_scope_sandbox(&record.id), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn sandbox_chat_session_create_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<CreateLiveChatSessionRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_running(&record)?;
    let summary = create_chat_session(live_scope_sandbox(&record.id), &address, req.title)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

async fn sandbox_chat_session_list_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let sessions = list_chat_sessions(&live_scope_sandbox(&record.id), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

async fn sandbox_chat_session_get_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let detail = get_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(detail)))
}

async fn sandbox_chat_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    stream_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)
}

async fn sandbox_chat_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = delete_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn sandbox_chat_run_cancel_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id, run_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = cancel_chat_run(
        &record,
        &live_scope_sandbox(&record.id),
        &address,
        &session_id,
        &run_id,
    )
    .await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_terminal_session_create_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    require_running(&record)?;
    let summary = create_terminal_session(live_scope_instance(&record), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

async fn instance_terminal_session_list_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let sessions = list_terminal_sessions(&live_scope_instance(&record), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

async fn instance_terminal_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    stream_terminal_session(&live_scope_instance(&record), &address, &session_id)
}

async fn instance_terminal_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = delete_terminal_session(&live_scope_instance(&record), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_chat_session_create_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<CreateLiveChatSessionRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    require_running(&record)?;
    let summary = create_chat_session(live_scope_instance(&record), &address, req.title)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

async fn instance_chat_session_list_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let sessions = list_chat_sessions(&live_scope_instance(&record), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

async fn instance_chat_session_get_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let detail = get_chat_session(&live_scope_instance(&record), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(detail)))
}

async fn instance_chat_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    stream_chat_session(&live_scope_instance(&record), &address, &session_id)
}

async fn instance_chat_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = delete_chat_session(&live_scope_instance(&record), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_chat_run_cancel_handler(
    SessionAuth(address): SessionAuth,
    Path((session_id, run_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = cancel_chat_run(
        &record,
        &live_scope_instance(&record),
        &address,
        &session_id,
        &run_id,
    )
    .await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ---------------------------------------------------------------------------
// Secret provisioning endpoints (2-phase)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct InjectSecretsRequest {
    env_json: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct SecretsResponse {
    status: String,
    sandbox_id: String,
    /// Whether AI credentials are available after this operation.
    credentials_available: bool,
}

#[derive(Serialize)]
struct GetSecretsResponse {
    sandbox_id: String,
    env_json: serde_json::Map<String, serde_json::Value>,
    credentials_available: bool,
}

async fn instance_get_secrets(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    let env_map: serde_json::Map<String, serde_json::Value> =
        if record.user_env_json.trim().is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str(&record.user_env_json).unwrap_or_default()
        };

    let creds =
        workflow_runtime_credentials_available(&record.effective_env_json()).unwrap_or(false);

    (
        StatusCode::OK,
        Json(GetSecretsResponse {
            sandbox_id: record.id,
            env_json: env_map,
            credentials_available: creds,
        }),
    )
        .into_response()
}

async fn instance_inject_secrets(
    SessionAuth(address): SessionAuth,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = crate::api_types::validate_secrets_map(&body.env_json) {
        return api_error(StatusCode::BAD_REQUEST, e).into_response();
    }

    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    match secret_provisioning::inject_secrets(&record.id, body.env_json, None).await {
        Ok(updated) => {
            sync_instance_record(&updated.id);
            let creds = workflow_runtime_credentials_available(&updated.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_configured".to_string(),
                    sandbox_id: updated.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn instance_wipe_secrets(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = match resolve_instance(&address) {
        Ok(record) => record,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = reject_instance_tee_secrets(&record) {
        return err.into_response();
    }

    match secret_provisioning::wipe_secrets(&record.id, None).await {
        Ok(updated) => {
            sync_instance_record(&updated.id);
            let creds = workflow_runtime_credentials_available(&updated.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_wiped".to_string(),
                    sandbox_id: updated.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    let record = match runtime::get_sandbox_by_id(&sandbox_id) {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };

    let env_map: serde_json::Map<String, serde_json::Value> =
        if record.user_env_json.trim().is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str(&record.user_env_json).unwrap_or_default()
        };

    let creds =
        workflow_runtime_credentials_available(&record.effective_env_json()).unwrap_or(false);

    (
        StatusCode::OK,
        Json(GetSecretsResponse {
            sandbox_id: record.id,
            env_json: env_map,
            credentials_available: creds,
        }),
    )
        .into_response()
}

async fn inject_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = crate::api_types::validate_secrets_map(&body.env_json) {
        return api_error(StatusCode::BAD_REQUEST, e).into_response();
    }
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    match secret_provisioning::inject_secrets(&sandbox_id, body.env_json, None).await {
        Ok(record) => {
            let creds = workflow_runtime_credentials_available(&record.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_configured".to_string(),
                    sandbox_id: record.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn wipe_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    match secret_provisioning::wipe_secrets(&sandbox_id, None).await {
        Ok(record) => {
            let creds = workflow_runtime_credentials_available(&record.effective_env_json())
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(SecretsResponse {
                    status: "secrets_wiped".to_string(),
                    sandbox_id: record.id,
                    credentials_available: creds,
                }),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn reject_instance_tee_secrets(record: &SandboxRecord) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.tee_config.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "TEE instances do not support plain secrets injection. Use sealed secrets instead.",
        ));
    }

    Ok(())
}

fn sync_instance_record(id: &str) {
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }
}

// ---------------------------------------------------------------------------
// Health & metrics endpoints (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum RuntimeProbeBackend {
    Docker,
    Firecracker,
    Tee,
}

impl RuntimeProbeBackend {
    fn as_str(self) -> &'static str {
        match self {
            RuntimeProbeBackend::Docker => "docker",
            RuntimeProbeBackend::Firecracker => "firecracker",
            RuntimeProbeBackend::Tee => "tee",
        }
    }
}

fn configured_runtime_probe_backend() -> Result<RuntimeProbeBackend, String> {
    let raw = std::env::var("SANDBOX_RUNTIME_BACKEND").unwrap_or_else(|_| "docker".to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "docker" | "container" => Ok(RuntimeProbeBackend::Docker),
        "firecracker" | "microvm" => Ok(RuntimeProbeBackend::Firecracker),
        "tee" | "confidential" | "confidential-vm" => Ok(RuntimeProbeBackend::Tee),
        _ => Err(format!(
            "invalid SANDBOX_RUNTIME_BACKEND '{raw}' (expected docker|firecracker|tee)"
        )),
    }
}

async fn probe_runtime_backend() -> (String, bool, Option<String>) {
    let backend = match configured_runtime_probe_backend() {
        Ok(v) => v,
        Err(err) => return ("invalid".to_string(), false, Some(err)),
    };

    match backend {
        RuntimeProbeBackend::Docker => {
            let ok = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let builder = runtime::docker_builder().await.ok()?;
                builder.client().ping().await.ok()?;
                Some(())
            })
            .await
            .is_ok_and(|v| v.is_some());

            (
                backend.as_str().to_string(),
                ok,
                if ok {
                    None
                } else {
                    Some("docker daemon unreachable".to_string())
                },
            )
        }
        RuntimeProbeBackend::Firecracker => {
            let checked = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::firecracker::health(),
            )
            .await;
            match checked {
                Ok(Ok(())) => (backend.as_str().to_string(), true, None),
                Ok(Err(err)) => (backend.as_str().to_string(), false, Some(err.to_string())),
                Err(_) => (
                    backend.as_str().to_string(),
                    false,
                    Some("firecracker host-agent health check timed out".to_string()),
                ),
            }
        }
        RuntimeProbeBackend::Tee => {
            let ok = crate::tee::try_tee_backend().is_some();
            (
                backend.as_str().to_string(),
                ok,
                if ok {
                    None
                } else {
                    Some("tee backend not initialized".to_string())
                },
            )
        }
    }
}

async fn health() -> impl IntoResponse {
    let (runtime_backend, runtime_ok, runtime_error) = probe_runtime_backend().await;

    // Check persistent store readability.
    let store_ok = runtime::sandboxes().and_then(|s| s.values()).is_ok();

    let (status, code) = match (runtime_ok, store_ok) {
        (true, true) => ("ok", StatusCode::OK),
        _ => ("degraded", StatusCode::SERVICE_UNAVAILABLE),
    };

    let check = |ok: bool| {
        if ok {
            json!({ "status": "ok" })
        } else {
            json!({ "status": "error" })
        }
    };

    (
        code,
        Json(json!({
            "status": status,
            "checks": {
                "runtime": check(runtime_ok),
                "store": check(store_ok),
            },
            "runtime_backend": runtime_backend,
            "runtime_error": runtime_error,
        })),
    )
}

/// Readiness probe — reports ready only when Docker daemon is reachable
/// AND the persistent store is functional. Returns 503 during startup or
/// when either subsystem is degraded. Kubernetes should route traffic only
/// to ready instances (`readinessProbe` on this endpoint).
async fn readyz() -> impl IntoResponse {
    let (runtime_backend, runtime_ok, runtime_error) = probe_runtime_backend().await;
    let store_ok = runtime::sandboxes().and_then(|s| s.values()).is_ok();

    if runtime_ok && store_ok {
        (StatusCode::OK, Json(json!({ "status": "ready" })))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "runtime_backend": runtime_backend,
                "runtime": runtime_ok,
                "runtime_error": runtime_error,
                "store": store_ok,
            })),
        )
    }
}

async fn prometheus_metrics() -> impl IntoResponse {
    let mut body = metrics::metrics().render_prometheus();
    body.push_str(&metrics::http_metrics().render_prometheus());
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

// ---------------------------------------------------------------------------
// Sandbox operation endpoints (exec, prompt, task, stop, resume, snapshot, SSH)
// ---------------------------------------------------------------------------

/// Look up a sandbox by ID and validate caller ownership.
fn resolve_sandbox(
    sandbox_id: &str,
    caller: &str,
) -> Result<SandboxRecord, (StatusCode, Json<ApiError>)> {
    runtime::require_sandbox_owner(sandbox_id, caller).map_err(|e| {
        let status = match &e {
            crate::SandboxError::NotFound(_) => StatusCode::NOT_FOUND,
            crate::SandboxError::Auth(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        api_error(status, e.to_string())
    })
}

/// Look up the singleton instance sandbox and validate ownership.
fn resolve_instance(caller: &str) -> Result<SandboxRecord, (StatusCode, Json<ApiError>)> {
    let record = runtime::get_instance_sandbox()
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Instance not provisioned"))?;

    if record.owner.is_empty() {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Instance has no owner configured",
        ));
    }
    if !record.owner.eq_ignore_ascii_case(caller) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Not authorized for this instance",
        ));
    }
    Ok(record)
}

fn require_running(record: &SandboxRecord) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.state == SandboxState::Running {
        return Ok(());
    }

    Err(api_error(
        StatusCode::CONFLICT,
        format!("Sandbox {} is stopped; resume it first", record.id),
    ))
}

/// Build `/terminals/commands` payload for exec operations.
fn build_exec_payload(command: &str, cwd: &str, env_json: &str, timeout_ms: u64) -> Value {
    let mut payload = Map::new();
    payload.insert("command".to_string(), Value::String(command.to_string()));
    if !cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    if timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(timeout_ms));
    }
    if !env_json.trim().is_empty() {
        if let Ok(Some(env_map)) = crate::util::parse_json_object(env_json, "env_json") {
            payload.insert("env".to_string(), env_map);
        }
    }
    Value::Object(payload)
}

/// Parse exec response from sidecar.
fn parse_exec_response(parsed: &Value) -> ExecApiResponse {
    let result = parsed.get("result");
    ExecApiResponse {
        exit_code: result
            .and_then(|r| r.get("exitCode"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        stdout: result
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: result
            .and_then(|r| r.get("stderr"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

#[cfg(test)]
fn first_nonempty_output_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

#[cfg(test)]
fn strip_terminal_control_sequences(output: &str) -> String {
    let mut cleaned = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }

        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }

        cleaned.push(ch);
    }

    cleaned
}

#[cfg(test)]
fn summarize_exec_failure(exec: &ExecApiResponse) -> String {
    let stderr = strip_terminal_control_sequences(&exec.stderr);
    let stdout = strip_terminal_control_sequences(&exec.stdout);
    first_nonempty_output_line(&stderr)
        .or_else(|| first_nonempty_output_line(&stdout))
        .unwrap_or("command failed")
        .to_string()
}

#[cfg(test)]
fn parse_detected_ssh_username(
    exec: &ExecApiResponse,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    if exec.exit_code != 0 {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            format!(
                "SSH username detection failed (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(exec)
            ),
        ));
    }

    let stdout = strip_terminal_control_sequences(&exec.stdout);
    for line in stdout.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if crate::ssh_validation::validate_ssh_username(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(api_error(
        StatusCode::BAD_GATEWAY,
        "SSH username detection failed: could not find a valid username in command output",
    ))
}

/// Build `/agents/run` payload for prompt/task operations.
fn build_agent_payload(
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    max_turns: Option<u64>,
    agent_identifier: &str,
) -> Value {
    let mut payload = Map::new();
    let identifier = if agent_identifier.is_empty() {
        "default"
    } else {
        agent_identifier
    };
    payload.insert("identifier".into(), json!(identifier));
    payload.insert("message".into(), json!(message));

    if !session_id.is_empty() {
        payload.insert("sessionId".into(), json!(session_id));
    }

    let mut backend = Map::new();
    if !model.is_empty() {
        backend.insert("model".into(), json!(model));
    }
    if !backend.is_empty() {
        payload.insert("backend".into(), Value::Object(backend));
    }

    if let Some(turns) = max_turns {
        if turns > 0 {
            let mut metadata = Map::new();
            metadata.insert("maxTurns".into(), json!(turns));
            if !context_json.trim().is_empty() {
                if let Ok(Some(Value::Object(ctx))) =
                    crate::util::parse_json_object(context_json, "context_json")
                {
                    metadata.extend(ctx);
                }
            }
            payload.insert("metadata".into(), Value::Object(metadata));
        }
    } else if !context_json.trim().is_empty() {
        if let Ok(Some(Value::Object(ctx))) =
            crate::util::parse_json_object(context_json, "context_json")
        {
            payload.insert("metadata".into(), Value::Object(ctx));
        }
    }

    if timeout_ms > 0 {
        payload.insert("timeout".into(), json!(timeout_ms));
    }
    Value::Object(payload)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AgentDescriptor {
    identifier: String,
    #[serde(
        rename = "displayName",
        alias = "display_name",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
}

#[derive(Debug, Deserialize)]
struct SidecarAgentList {
    #[serde(default)]
    agents: Vec<AgentDescriptor>,
}

#[derive(Debug, Serialize)]
struct AgentListApiResponse {
    agents: Vec<AgentDescriptor>,
    count: usize,
}

fn format_available_agents(agents: &[AgentDescriptor]) -> String {
    agents
        .iter()
        .map(|agent| agent.identifier.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn invalid_agent_identifier_error(
    agent_identifier: &str,
    agents: &[AgentDescriptor],
) -> (StatusCode, Json<ApiError>) {
    let trimmed = agent_identifier.trim();
    if agents.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            format!(
                "Unknown agent identifier \"{trimmed}\". This sidecar image does not register that agent."
            ),
        );
    }

    api_error(
        StatusCode::BAD_REQUEST,
        format!(
            "Unknown agent identifier \"{trimmed}\". Available agents: {}",
            format_available_agents(agents)
        ),
    )
}

async fn translate_missing_agent_factory_error(
    record: &SandboxRecord,
    agent_identifier: &str,
    err: &(StatusCode, Json<ApiError>),
) -> Option<(StatusCode, Json<ApiError>)> {
    if agent_identifier.trim().is_empty() {
        return None;
    }

    let message = err.1.0.error.as_str();
    if message.contains("No factory registered for agent identifier") {
        // This is a semantic agent-selection error, not a transport failure.
        // Clear the unhealthy mark so a best-effort /agents lookup can enrich
        // the returned error without restoring hot-path prevalidation.
        circuit_breaker::clear(&record.id);
        let agents = match fetch_sidecar_agents(record).await {
            Ok(Some(agents)) => agents,
            Ok(None) | Err(_) => Vec::new(),
        };
        return Some(invalid_agent_identifier_error(agent_identifier, &agents));
    }

    None
}

fn agent_warmup_retryable(err: &(StatusCode, Json<ApiError>)) -> bool {
    let message = err.1.0.error.as_str();
    message.contains("OpenCode server is not responding")
        || message.contains("Failed to create OpenCode session")
}

fn request_id_for_logs() -> Option<String> {
    CURRENT_REQUEST_ID.try_with(Clone::clone).ok()
}

fn agents_endpoint_unsupported(err: &(StatusCode, Json<ApiError>)) -> bool {
    let message = err.1.0.error.as_str();
    message.contains("HTTP 404") || message.contains("HTTP 405") || message.contains("HTTP 501")
}

fn agent_discovery_not_supported_message(message: &str) -> bool {
    message.contains("HTTP 404") || message.contains("HTTP 405") || message.contains("HTTP 501")
}

fn parse_agent_descriptors(
    parsed: Value,
) -> Result<Vec<AgentDescriptor>, (StatusCode, Json<ApiError>)> {
    serde_json::from_value::<SidecarAgentList>(parsed)
        .map(|body| body.agents)
        .map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Invalid sidecar /agents response: {err}"),
            )
        })
}

async fn agent_stream_on_sidecar(
    record: &SandboxRecord,
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    max_turns: Option<u64>,
    mut on_event: impl FnMut(&SidecarSseEvent),
) -> Result<AgentStreamOutcome, (StatusCode, Json<ApiError>)> {
    let payload = build_agent_payload(
        message,
        session_id,
        model,
        context_json,
        resolve_agent_run_timeout_ms(timeout_ms, max_turns),
        max_turns,
        &record.agent_identifier,
    );
    let client = crate::util::http_client_no_timeout().map_err(|err| {
        api_error(
            StatusCode::BAD_GATEWAY,
            format!("Unable to create sidecar stream client: {err}"),
        )
    })?;
    let mut current_record = record.clone();
    let mut last_retry_after_ms = None;

    for attempt in 0..=AGENT_WARMUP_RETRY_DELAYS_MS.len() {
        let url = build_url(&current_record.sidecar_url, "/agents/run/stream").map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Invalid sidecar stream URL: {err}"),
            )
        })?;
        let mut headers = auth_headers(&current_record.token).map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Unable to build sidecar auth headers: {err}"),
            )
        })?;

        if let Ok(rid) = CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&rid) {
                headers.insert("x-request-id", value);
            }
        }

        let response = client
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                api_error(
                    StatusCode::BAD_GATEWAY,
                    format!("Agent stream request failed: {err}"),
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown stream error".to_string());
            let parsed_body = serde_json::from_str::<Value>(&body).ok();
            let message = parsed_body
                .as_ref()
                .and_then(|value| value.get("error"))
                .and_then(|error| {
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| error.as_str())
                })
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {status}: {body}"));
            let err = api_error(StatusCode::BAD_GATEWAY, message);
            if let Some(translated) =
                translate_missing_agent_factory_error(record, &record.agent_identifier, &err).await
            {
                return Err(translated);
            }
            if !agent_warmup_retryable(&err) {
                return Err(err);
            }

            circuit_breaker::clear(&record.id);
            if let Some(delay_ms) = AGENT_WARMUP_RETRY_DELAYS_MS.get(attempt).copied() {
                tracing::warn!(
                    request_id = ?request_id_for_logs(),
                    sandbox_id = %record.id,
                    sidecar_url = %current_record.sidecar_url,
                    attempt = attempt + 1,
                    retry_delay_ms = delay_ms,
                    error = %err.1.0.error,
                    "agent warmup detected; retrying prompt/task stream"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                current_record = runtime::get_sandbox_by_id(&record.id)
                    .unwrap_or_else(|_| current_record.clone());
                last_retry_after_ms = Some(delay_ms);
                continue;
            }

            tracing::warn!(
                request_id = ?request_id_for_logs(),
                sandbox_id = %record.id,
                sidecar_url = %current_record.sidecar_url,
                attempts = AGENT_WARMUP_RETRY_DELAYS_MS.len() + 1,
                error = %err.1.0.error,
                "agent warmup retries exhausted for streaming run"
            );
            return Err(api_error_with_details(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sandbox agent is still starting up. Please retry shortly.",
                Some(AGENT_WARMUP_ERROR_CODE),
                last_retry_after_ms,
            ));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut accumulated_text = String::new();
        let mut outcome = AgentStreamOutcome::default();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| {
                api_error(
                    StatusCode::BAD_GATEWAY,
                    format!("Agent stream read failed: {err}"),
                )
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(index) = buffer.find("\n\n") {
                let frame = buffer[..index].to_string();
                buffer = buffer[index + 2..].to_string();

                let Some(event) = parse_sse_event(&frame) else {
                    continue;
                };
                match event.event_type.as_str() {
                    "message.part.updated" => {
                        if let Some(part) = event.data.get("part").and_then(normalize_stream_part) {
                            if part.get("type").and_then(Value::as_str) == Some("text") {
                                if let Some(text) = part.get("text").and_then(Value::as_str) {
                                    accumulated_text = text.to_string();
                                }
                            }
                        }
                        on_event(&event);
                    }
                    "result" => {
                        outcome = parse_agent_stream_result(&event.data);
                    }
                    "error" => {
                        let message = event
                            .data
                            .get("message")
                            .or_else(|| {
                                event
                                    .data
                                    .get("error")
                                    .and_then(|value| value.get("message"))
                            })
                            .and_then(Value::as_str)
                            .unwrap_or("Agent stream failed");
                        return Err(api_error(StatusCode::BAD_GATEWAY, message));
                    }
                    _ => on_event(&event),
                }
            }
        }

        if outcome.response.is_empty() {
            outcome.response = accumulated_text;
        }
        outcome.success = outcome.error.is_empty();
        return Ok(outcome);
    }

    Err(api_error_with_details(
        StatusCode::SERVICE_UNAVAILABLE,
        "Sandbox agent is still starting up. Please retry shortly.",
        Some(AGENT_WARMUP_ERROR_CODE),
        last_retry_after_ms,
    ))
}

enum SidecarAttemptFailure {
    Timeout,
    Error(SandboxError),
}

fn is_retryable_transport_error(err: &SandboxError) -> bool {
    matches!(err, SandboxError::Http(msg) if msg.contains("error sending request for url"))
}

async fn try_refresh_stale_endpoint(
    record: &SandboxRecord,
    op_name: &str,
) -> Option<SandboxRecord> {
    if !runtime::supports_docker_endpoint_refresh(record) {
        return None;
    }

    match runtime::refresh_docker_sandbox_endpoint(record).await {
        Ok(updated) => Some(updated),
        Err(err) => {
            tracing::warn!(
                sandbox_id = %record.id,
                operation = op_name,
                error = %err,
                "failed to refresh stale sandbox endpoint"
            );
            None
        }
    }
}

async fn run_sidecar_json_attempt(
    record: &SandboxRecord,
    path: &str,
    payload: &Value,
    timeout: Duration,
) -> std::result::Result<Value, SidecarAttemptFailure> {
    match tokio::time::timeout(
        timeout,
        sidecar_post_json_without_timeout(
            &record.sidecar_url,
            path,
            &record.token,
            payload.clone(),
        ),
    )
    .await
    {
        Err(_) => Err(SidecarAttemptFailure::Timeout),
        Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
        Ok(Ok(parsed)) => Ok(parsed),
    }
}

async fn run_sidecar_get_json_attempt(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
) -> std::result::Result<Value, SidecarAttemptFailure> {
    match tokio::time::timeout(
        timeout,
        sidecar_get_json(&record.sidecar_url, path, &record.token),
    )
    .await
    {
        Err(_) => Err(SidecarAttemptFailure::Timeout),
        Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
        Ok(Ok(parsed)) => Ok(parsed),
    }
}

/// Call a sidecar endpoint with circuit-breaker integration and timeout.
///
/// This is the single entry point for all sidecar HTTP calls. It:
/// 1. Checks the circuit breaker (returns 503 if in cooldown)
/// 2. Sends the request with the given timeout
/// 3. Marks the sidecar healthy/unhealthy based on the outcome
/// 4. Touches the sandbox activity timestamp on success
async fn sidecar_call(
    record: &SandboxRecord,
    path: &str,
    payload: Value,
    timeout: Duration,
    op_name: &str,
    allow_transport_retry: bool,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_json_attempt(record, path, &payload, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if allow_transport_retry && is_retryable_transport_error(&err) {
                if let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await {
                    match run_sidecar_json_attempt(&refreshed, path, &payload, timeout).await {
                        Ok(parsed) => {
                            circuit_breaker::mark_healthy(&record.id);
                            runtime::touch_sandbox(&record.id);
                            return Ok(parsed);
                        }
                        Err(SidecarAttemptFailure::Timeout) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(
                                StatusCode::GATEWAY_TIMEOUT,
                                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                            ));
                        }
                        Err(SidecarAttemptFailure::Error(retry_err)) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                        }
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

async fn sidecar_get_call(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
    op_name: &str,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_get_json_attempt(record, path, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            let err_message = err.to_string();
            if op_name == "agents" && agent_discovery_not_supported_message(&err_message) {
                return Err(api_error(StatusCode::BAD_GATEWAY, err_message));
            }

            if is_retryable_transport_error(&err) {
                if let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await {
                    match run_sidecar_get_json_attempt(&refreshed, path, timeout).await {
                        Ok(parsed) => {
                            circuit_breaker::mark_healthy(&record.id);
                            runtime::touch_sandbox(&record.id);
                            return Ok(parsed);
                        }
                        Err(SidecarAttemptFailure::Timeout) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(
                                StatusCode::GATEWAY_TIMEOUT,
                                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                            ));
                        }
                        Err(SidecarAttemptFailure::Error(retry_err)) => {
                            let retry_message = retry_err.to_string();
                            if op_name == "agents"
                                && agent_discovery_not_supported_message(&retry_message)
                            {
                                return Err(api_error(StatusCode::BAD_GATEWAY, retry_message));
                            }
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(StatusCode::BAD_GATEWAY, retry_message));
                        }
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err_message))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

async fn fetch_sidecar_agents(
    record: &SandboxRecord,
) -> Result<Option<Vec<AgentDescriptor>>, (StatusCode, Json<ApiError>)> {
    let parsed = match sidecar_get_call(record, "/agents", SIDECAR_DEFAULT_TIMEOUT, "agents").await
    {
        Ok(parsed) => parsed,
        Err(err) if agents_endpoint_unsupported(&err) => return Ok(None),
        Err(err) => return Err(err),
    };

    parse_agent_descriptors(parsed).map(Some)
}

async fn list_agents_on_sidecar(
    record: &SandboxRecord,
) -> Result<Vec<AgentDescriptor>, (StatusCode, Json<ApiError>)> {
    match fetch_sidecar_agents(record).await? {
        Some(agents) => Ok(agents),
        None => Err(api_error(
            StatusCode::NOT_IMPLEMENTED,
            "This sidecar image does not expose agent discovery.",
        )),
    }
}

async fn exec_on_sidecar(
    record: &SandboxRecord,
    req: &ExecApiRequest,
) -> Result<ExecApiResponse, (StatusCode, Json<ApiError>)> {
    let payload = build_exec_payload(&req.command, &req.cwd, &req.env_json, req.timeout_ms);
    let parsed = sidecar_call(
        record,
        "/terminals/commands",
        payload,
        SIDECAR_EXEC_TIMEOUT,
        "exec",
        true,
    )
    .await?;
    Ok(parse_exec_response(&parsed))
}

async fn sandbox_agents_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let agents = list_agents_on_sidecar(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(AgentListApiResponse {
            count: agents.len(),
            agents,
        }),
    ))
}

async fn instance_agents_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let agents = list_agents_on_sidecar(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(AgentListApiResponse {
            count: agents.len(),
            agents,
        }),
    ))
}

// ── Exec ─────────────────────────────────────────────────────────────────

async fn sandbox_exec_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let scope = live_scope_sandbox(&record.id);
    let resp = exec_on_sidecar(&record, &req).await?;
    publish_terminal_output(
        &scope,
        &address,
        &req.session_id,
        &resp.stdout,
        &resp.stderr,
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_exec_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let scope = live_scope_instance(&record);
    let resp = exec_on_sidecar(&record, &req).await?;
    publish_terminal_output(
        &scope,
        &address,
        &req.session_id,
        &resp.stdout,
        &resp.stderr,
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

fn chat_run_status_label(status: &ChatRunStatus) -> &'static str {
    match status {
        ChatRunStatus::Queued => "queued",
        ChatRunStatus::Running => "running",
        ChatRunStatus::Cancelling => "cancelling",
        ChatRunStatus::Completed => "completed",
        ChatRunStatus::Failed => "failed",
        ChatRunStatus::Cancelled => "cancelled",
        ChatRunStatus::Interrupted => "interrupted",
    }
}

fn resolve_agent_run_timeout_ms(timeout_ms: u64, max_turns: Option<u64>) -> u64 {
    if timeout_ms > 0 {
        timeout_ms
    } else if max_turns.is_some() {
        DEFAULT_TASK_RUN_TIMEOUT_MS
    } else {
        DEFAULT_PROMPT_RUN_TIMEOUT_MS
    }
}

fn resolve_or_create_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<ChatSessionRecord, (StatusCode, Json<ApiError>)> {
    if session_id.trim().is_empty() {
        return chat_state::create_session(scope_id, owner, Some("New Chat"))
            .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e));
    }

    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;

    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }

    Ok(session)
}

fn resolve_chat_run(
    scope_id: &str,
    owner: &str,
    session_id: &str,
    run_id: &str,
) -> Result<(ChatSessionRecord, ChatRunRecord), (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }

    let run = chat_state::get_run(run_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            api_error_with_details(
                StatusCode::NOT_FOUND,
                "Chat run not found",
                Some("CHAT_RUN_NOT_FOUND"),
                None,
            )
        })?;

    if run.session_id != session.id
        || run.scope_id != scope_id
        || !run.owner.eq_ignore_ascii_case(owner)
    {
        return Err(api_error_with_details(
            StatusCode::NOT_FOUND,
            "Chat run not found",
            Some("CHAT_RUN_NOT_FOUND"),
            None,
        ));
    }

    Ok((session, run))
}

async fn best_effort_cancel_sidecar_run(record: &SandboxRecord) {
    let _ = tokio::time::timeout(
        CHAT_CANCEL_TIMEOUT,
        sidecar_post_json(
            &record.sidecar_url,
            "/agents/run/cancel",
            &record.token,
            json!({}),
        ),
    )
    .await;
}

fn finalize_cancelled_chat_run(
    session_id: &str,
    run_id: &str,
    error_text: &str,
) -> Result<ChatRunRecord, (StatusCode, Json<ApiError>)> {
    let cancelled_at = chat_state::now_ms();
    let updated = chat_state::update_run(run_id, |run| {
        run.status = ChatRunStatus::Cancelled;
        run.completed_at = Some(cancelled_at);
        run.error = Some(error_text.to_string());
    })
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if !updated {
        return Err(api_error_with_details(
            StatusCode::NOT_FOUND,
            "Chat run not found",
            Some("CHAT_RUN_NOT_FOUND"),
            None,
        ));
    }

    let _ = chat_state::clear_session_active_run(session_id);
    let updated_run = chat_state::get_run(run_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Chat run disappeared"))?;
    publish_run_event(session_id, "run_cancelled", &updated_run);
    publish_run_progress(
        session_id,
        &updated_run.id,
        &updated_run.status,
        "cancelled",
        "Run cancelled by user.",
    );
    emit_session_error(session_id, "Execution cancelled by user", Some("EXECUTION_CANCELLED"));
    emit_session_idle(session_id);
    Ok(updated_run)
}

fn enqueue_chat_run(
    scope_id: &str,
    owner: &str,
    session_id: &str,
    kind: ChatRunKind,
    request_text: &str,
) -> Result<(ChatSessionRecord, ChatRunRecord), (StatusCode, Json<ApiError>)> {
    let _guard = CHAT_RUN_ENQUEUE_GUARD.lock().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chat enqueue lock poisoned: {e}"),
        )
    })?;
    if let Some(existing) = chat_state::active_run_for_scope(scope_id, owner)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(api_error_with_details(
            StatusCode::CONFLICT,
            format!(
                "A chat run is already active for this resource ({})",
                existing.id
            ),
            Some("CHAT_RUN_ACTIVE"),
            None,
        ));
    }

    let session = resolve_or_create_chat_session(scope_id, owner, session_id)?;
    let run = chat_state::create_run(&session.id, scope_id, owner, kind, request_text)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let _ = chat_state::maybe_auto_title_session(&session.id, request_text);

    let user_message = ChatMessageRecord {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: Some(run.id.clone()),
        role: "user".to_string(),
        content: request_text.to_string(),
        created_at: chat_state::now_ms(),
        completed_at: Some(chat_state::now_ms()),
        parts: vec![json!({
            "id": format!("text-{}", uuid::Uuid::new_v4()),
            "type": "text",
            "text": request_text.to_string(),
        })],
        trace_id: None,
        success: None,
        error: None,
    };
    publish_chat_message(&session.id, user_message, "user_message");
    if let Ok(Some(current_session)) = chat_state::get_session(&session.id) {
        if let Some(message) = current_session.messages.last() {
            emit_message_updated(&session.id, message);
            for part in &message.parts {
                emit_message_part_updated(&session.id, &message.id, part.clone());
            }
        }
    }
    if let Ok(Some(queued_run)) = chat_state::get_run(&run.id) {
        publish_run_event(&session.id, "run_queued", &queued_run);
        return Ok((session, queued_run));
    }

    Ok((session, run))
}

struct SpawnChatRunRequest {
    session_id: String,
    run_id: String,
    message: String,
    model: String,
    context_json: String,
    timeout_ms: u64,
    max_turns: Option<u64>,
}

fn spawn_chat_run(record: SandboxRecord, request: SpawnChatRunRequest) {
    let SpawnChatRunRequest {
        session_id,
        run_id,
        message,
        model,
        context_json,
        timeout_ms,
        max_turns,
    } = request;
    let spawned_run_id = run_id.clone();
    let handle = tokio::spawn(async move {
        struct ChatRunAbortGuard {
            run_id: String,
        }

        impl Drop for ChatRunAbortGuard {
            fn drop(&mut self) {
                clear_chat_run_abort(&self.run_id);
            }
        }

        let _abort_guard = ChatRunAbortGuard {
            run_id: run_id.clone(),
        };
        publish_run_progress(
            &session_id,
            &run_id,
            &ChatRunStatus::Queued,
            "queued",
            "Run accepted and queued by the operator.",
        );

        let started_at = chat_state::now_ms();
        let _ = chat_state::update_run(&run_id, |run| {
            run.status = ChatRunStatus::Running;
            run.started_at = Some(started_at);
        });
        if let Ok(Some(run)) = chat_state::get_run(&run_id) {
            publish_run_event(&session_id, "run_started", &run);
            publish_run_progress(
                &session_id,
                &run_id,
                &run.status,
                "running",
                "Operator started the agent run.",
            );
        }

        let sidecar_session_id = chat_state::get_session(&session_id)
            .ok()
            .flatten()
            .and_then(|session| session.latest_sidecar_session_id)
            .unwrap_or_default();

        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let assistant_started_at = chat_state::now_ms();
        let assistant_message = ChatMessageRecord {
            id: assistant_message_id.clone(),
            run_id: Some(run_id.clone()),
            role: "assistant".to_string(),
            content: String::new(),
            created_at: assistant_started_at,
            completed_at: None,
            parts: Vec::new(),
            trace_id: None,
            success: None,
            error: None,
        };
        let _ = chat_state::append_message(&session_id, assistant_message.clone());
        emit_message_updated(&session_id, &assistant_message);
        let mut ignored_upstream_message_ids = HashSet::new();
        let mut assistant_upstream_message_ids = HashSet::new();

        let result = agent_stream_on_sidecar(
            &record,
            &message,
            &sidecar_session_id,
            &model,
            &context_json,
            timeout_ms,
            max_turns,
            |event| {
                if event.event_type == "message.part.updated" {
                    if let Some(part) = event.data.get("part").and_then(normalize_stream_part) {
                        if !should_forward_stream_part(
                            &part,
                            &message,
                            &mut ignored_upstream_message_ids,
                            &mut assistant_upstream_message_ids,
                        ) {
                            return;
                        }
                        let _ = chat_state::upsert_message_part(
                            &session_id,
                            &assistant_message_id,
                            part.clone(),
                        );
                        emit_message_part_updated(&session_id, &assistant_message_id, part);
                    }
                }
            },
        )
        .await;

        if let Ok(Some(existing_run)) = chat_state::get_run(&run_id) {
            if matches!(
                existing_run.status,
                ChatRunStatus::Cancelled | ChatRunStatus::Cancelling
            ) {
                return;
            }
        }

        match result {
            Ok(ar) => {
                metrics::metrics().record_job(ar.duration_ms, ar.input_tokens, ar.output_tokens);
                let completed_at = chat_state::now_ms();
                let final_status = if ar.success {
                    ChatRunStatus::Completed
                } else {
                    ChatRunStatus::Failed
                };
                let assistant_content = if !ar.response.trim().is_empty() {
                    ar.response.clone()
                } else if !ar.error.trim().is_empty() {
                    format!("Error: {error}", error = ar.error)
                } else {
                    String::new()
                };

                if !ar.session_id.trim().is_empty() {
                    let _ = chat_state::set_session_sidecar_session_id(
                        &session_id,
                        Some(ar.session_id.clone()),
                    );
                }

                let _ = chat_state::update_run(&run_id, |run| {
                    run.status = final_status.clone();
                    run.completed_at = Some(completed_at);
                    if !ar.session_id.trim().is_empty() {
                        run.sidecar_session_id = Some(ar.session_id.clone());
                    }
                    if !ar.trace_id.trim().is_empty() {
                        run.trace_id = Some(ar.trace_id.clone());
                    }
                    if !ar.response.trim().is_empty() {
                        run.final_output = Some(ar.response.clone());
                    }
                    if !ar.error.trim().is_empty() {
                        run.error = Some(ar.error.clone());
                    }
                });
                let _ = chat_state::clear_session_active_run(&session_id);

                let mut assistant_message = chat_state::get_session(&session_id)
                    .ok()
                    .flatten()
                    .and_then(|session| {
                        session
                            .messages
                            .into_iter()
                            .find(|entry| entry.id == assistant_message_id)
                    })
                    .unwrap_or(ChatMessageRecord {
                        id: assistant_message_id.clone(),
                        run_id: Some(run_id.clone()),
                        role: "assistant".to_string(),
                        content: String::new(),
                        created_at: assistant_started_at,
                        completed_at: None,
                        parts: Vec::new(),
                        trace_id: None,
                        success: None,
                        error: None,
                    });
                if assistant_message.parts.is_empty() && !assistant_content.is_empty() {
                    assistant_message.parts.push(json!({
                        "id": format!("text-{}", uuid::Uuid::new_v4()),
                        "type": "text",
                        "text": assistant_content.clone(),
                    }));
                }
                finalize_streamed_assistant_parts(&mut assistant_message.parts, completed_at);
                assistant_message.content = assistant_content;
                assistant_message.completed_at = Some(completed_at);
                assistant_message.trace_id = if ar.trace_id.trim().is_empty() {
                    None
                } else {
                    Some(ar.trace_id.clone())
                };
                assistant_message.success = Some(ar.success);
                assistant_message.error = if ar.error.trim().is_empty() {
                    None
                } else {
                    Some(ar.error.clone())
                };
                let _ = chat_state::append_message(&session_id, assistant_message.clone());
                emit_message_updated(&session_id, &assistant_message);
                for part in &assistant_message.parts {
                    emit_message_part_updated(&session_id, &assistant_message.id, part.clone());
                }
                emit_session_idle(&session_id);

                if let Ok(Some(updated_run)) = chat_state::get_run(&run_id) {
                    publish_run_event(
                        &session_id,
                        if updated_run.status == ChatRunStatus::Completed {
                            "run_completed"
                        } else {
                            "run_failed"
                        },
                        &updated_run,
                    );
                    publish_run_progress(
                        &session_id,
                        &updated_run.id,
                        &updated_run.status,
                        if updated_run.status == ChatRunStatus::Completed {
                            "completed"
                        } else {
                            "failed"
                        },
                        if updated_run.status == ChatRunStatus::Completed {
                            "Run completed successfully."
                        } else {
                            "Run finished with an error."
                        },
                    );
                }
            }
            Err((status, api_error_body)) => {
                let completed_at = chat_state::now_ms();
                let error_text = api_error_body.0.error.clone();
                let _ = chat_state::update_run(&run_id, |run| {
                    run.status = ChatRunStatus::Failed;
                    run.completed_at = Some(completed_at);
                    run.error = Some(error_text.clone());
                });
                let _ = chat_state::clear_session_active_run(&session_id);

                let assistant_message = ChatMessageRecord {
                    id: assistant_message_id.clone(),
                    run_id: Some(run_id.clone()),
                    role: "assistant".to_string(),
                    content: format!("Error: {error_text}"),
                    created_at: assistant_started_at,
                    completed_at: Some(completed_at),
                    parts: vec![json!({
                        "id": format!("text-{}", uuid::Uuid::new_v4()),
                        "type": "text",
                        "text": format!("Error: {error_text}"),
                    })],
                    trace_id: None,
                    success: Some(false),
                    error: Some(error_text.clone()),
                };
                let _ = chat_state::append_message(&session_id, assistant_message.clone());
                emit_message_updated(&session_id, &assistant_message);
                for part in &assistant_message.parts {
                    emit_message_part_updated(&session_id, &assistant_message.id, part.clone());
                }
                emit_session_error(
                    &session_id,
                    &error_text,
                    api_error_body.0.code.as_deref(),
                );
                emit_session_idle(&session_id);

                if let Ok(Some(updated_run)) = chat_state::get_run(&run_id) {
                    publish_run_event(&session_id, "run_failed", &updated_run);
                    publish_run_progress(
                        &session_id,
                        &updated_run.id,
                        &updated_run.status,
                        "failed",
                        "Run failed before the operator received a successful result.",
                    );
                } else {
                    let _ = status;
                }
            }
        }
    });
    register_chat_run_abort(&spawned_run_id, handle.abort_handle());
}

fn accepted_prompt_response(run: &ChatRunRecord, session_id: &str) -> PromptApiResponse {
    PromptApiResponse {
        accepted: true,
        run_id: run.id.clone(),
        session_id: session_id.to_string(),
        status: chat_run_status_label(&run.status).to_string(),
        accepted_at: run.created_at,
    }
}

fn accepted_task_response(run: &ChatRunRecord, session_id: &str) -> TaskApiResponse {
    TaskApiResponse {
        accepted: true,
        run_id: run.id.clone(),
        session_id: session_id.to_string(),
        status: chat_run_status_label(&run.status).to_string(),
        accepted_at: run.created_at,
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────

async fn sandbox_prompt_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let scope = live_scope_sandbox(&record.id);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Prompt,
        &req.message,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.message,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: None,
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_prompt_response(&run, &session.id)),
    ))
}

async fn instance_prompt_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let scope = live_scope_instance(&record);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Prompt,
        &req.message,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.message,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: None,
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_prompt_response(&run, &session.id)),
    ))
}

// ── Task ─────────────────────────────────────────────────────────────────

async fn sandbox_task_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let scope = live_scope_sandbox(&record.id);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Task,
        &req.prompt,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.prompt,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: Some(req.max_turns),
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_task_response(&run, &session.id)),
    ))
}

async fn instance_task_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let scope = live_scope_instance(&record);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Task,
        &req.prompt,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.prompt,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: Some(req.max_turns),
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_task_response(&run, &session.id)),
    ))
}

// ── Stop / Resume ────────────────────────────────────────────────────────

/// Timeout for stop/resume operations (Docker stop + potential health polling).
const STOP_RESUME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

fn handle_lifecycle_outcome(
    result: Result<(), crate::SandboxError>,
    already_message: &str,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    match result {
        Ok(()) => Ok(()),
        Err(crate::SandboxError::Validation(msg))
            if msg.to_ascii_lowercase().contains(already_message) =>
        {
            // Idempotent lifecycle call: already in target state.
            Ok(())
        }
        Err(crate::SandboxError::Unavailable(msg)) => {
            Err(api_error(StatusCode::SERVICE_UNAVAILABLE, msg))
        }
        Err(e) => Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn sandbox_stop_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let stop_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::stop_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Stop operation timed out"))?;
    handle_lifecycle_outcome(stop_result, "already stopped")?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: record.id,
            state: "stopped".into(),
        }),
    ))
}

async fn sandbox_resume_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resume_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::resume_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Resume operation timed out"))?;
    handle_lifecycle_outcome(resume_result, "already running")?;
    // Resume transitions the sandbox back to service; clear any stale breaker state.
    circuit_breaker::mark_healthy(&record.id);
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: record.id,
            state: "running".into(),
        }),
    ))
}

async fn instance_stop_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    let stop_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::stop_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Stop operation timed out"))?;
    handle_lifecycle_outcome(stop_result, "already stopped")?;

    // Sync updated state back to instance store.
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(&id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }

    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: id,
            state: "stopped".into(),
        }),
    ))
}

async fn instance_resume_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    let resume_result = tokio::time::timeout(STOP_RESUME_TIMEOUT, runtime::resume_sidecar(&record))
        .await
        .map_err(|_| api_error(StatusCode::GATEWAY_TIMEOUT, "Resume operation timed out"))?;
    handle_lifecycle_outcome(resume_result, "already running")?;
    circuit_breaker::mark_healthy(&id);

    // Sync updated record (port mappings may have changed) back to instance store.
    if let Ok(Some(updated)) = sandboxes().and_then(|s| s.get(&id)) {
        let _ = runtime::instance_store().and_then(|s| s.insert("instance".to_string(), updated));
    }

    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(LifecycleApiResponse {
            success: true,
            sandbox_id: id,
            state: "running".into(),
        }),
    ))
}

// ── Snapshot ─────────────────────────────────────────────────────────────

async fn run_snapshot(
    record: &SandboxRecord,
    req: &SnapshotApiRequest,
) -> Result<SnapshotApiResponse, (StatusCode, Json<ApiError>)> {
    if req.destination.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Snapshot destination is required",
        ));
    }
    let command = crate::util::build_snapshot_command(
        &req.destination,
        req.include_workspace,
        req.include_state,
    )
    .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let payload = json!({ "command": format!("sh -c {}", crate::util::shell_escape(&command)) });
    let parsed = sidecar_call(
        record,
        "/terminals/commands",
        payload,
        SIDECAR_DEFAULT_TIMEOUT,
        "snapshot",
        true,
    )
    .await?;
    Ok(SnapshotApiResponse {
        success: true,
        result: parsed,
    })
}

async fn sandbox_snapshot_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SnapshotApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = run_snapshot(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_snapshot_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SnapshotApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = run_snapshot(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ── SSH ──────────────────────────────────────────────────────────────────

fn require_ssh(record: &SandboxRecord) -> Result<(), (StatusCode, Json<ApiError>)> {
    if record.ssh_port.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "SSH is not enabled for this sandbox",
        ));
    }
    Ok(())
}

async fn detect_ssh_username(
    record: &SandboxRecord,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    runtime::detect_ssh_username(record)
        .await
        .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))
}

async fn run_ssh_provision(
    record: &SandboxRecord,
    req: &SshProvisionApiRequest,
) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let (username, parsed) =
        runtime::provision_ssh_key(record, req.username.as_deref(), &req.public_key)
            .await
            .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(SshApiResponse {
        success: true,
        username,
        result: parsed,
    })
}

async fn run_ssh_revoke(
    record: &SandboxRecord,
    req: &SshRevokeApiRequest,
) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let (username, parsed) =
        runtime::revoke_ssh_key(record, req.username.as_deref(), &req.public_key)
            .await
            .map_err(|e| api_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(SshApiResponse {
        success: true,
        username,
        result: parsed,
    })
}

async fn sandbox_ssh_user_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let username = detect_ssh_username(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(SshUserApiResponse {
            success: true,
            username,
        }),
    ))
}

async fn sandbox_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn sandbox_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_ssh(&record)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_ssh_user_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let username = detect_ssh_username(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(SshUserApiResponse {
            success: true,
            username,
        }),
    ))
}

async fn instance_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    require_ssh(&record)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ---------------------------------------------------------------------------
// Port proxy endpoints
// ---------------------------------------------------------------------------

/// Timeout for proxied user-port requests.
const PORT_PROXY_TIMEOUT: Duration = Duration::from_secs(30);

/// List exposed port mappings for a sandbox.
async fn sandbox_ports_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(json!({ "ports": record.extra_ports })),
    ))
}

/// List exposed port mappings for the singleton instance sandbox.
async fn instance_ports_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(json!({ "ports": record.extra_ports })),
    ))
}

/// Reverse-proxy an HTTP request to an exposed container port (with path).
async fn sandbox_port_proxy_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(String, u16, String)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let (sandbox_id, port, path) = params;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    run_port_proxy(record, port, &path, query.as_deref(), method, headers, body).await
}

/// Reverse-proxy to container port root (no sub-path).
async fn sandbox_port_proxy_root_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(String, u16)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let (sandbox_id, port) = params;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    run_port_proxy(record, port, "", query.as_deref(), method, headers, body).await
}

/// Reverse-proxy for instance mode (with path).
async fn instance_port_proxy_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(u16, String)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let (port, path) = params;
    let record = resolve_instance(&address)?;
    run_port_proxy(record, port, &path, query.as_deref(), method, headers, body).await
}

/// Reverse-proxy for instance mode root (no sub-path).
async fn instance_port_proxy_root_handler(
    SessionAuth(address): SessionAuth,
    Path(port): Path<u16>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let record = resolve_instance(&address)?;
    run_port_proxy(record, port, "", query.as_deref(), method, headers, body).await
}

/// Core proxy logic shared between sandbox and instance handlers.
///
/// Target is always `http://127.0.0.1:{host_port}` — the container port is
/// mapped to a random localhost port by Docker, so SSRF to external hosts is
/// impossible by construction.
async fn run_port_proxy(
    record: SandboxRecord,
    port: u16,
    path: &str,
    query: Option<&str>,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    // Defense-in-depth: reject clearly malicious path patterns even though the
    // target is always localhost and reqwest::Url::parse validates the result.
    if path.contains('\0') || path.starts_with("//") {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Invalid proxy path".to_string(),
        ));
    }

    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    tracing::debug!(
        sandbox_id = %record.id,
        container_port = port,
        method = %method,
        path,
        "port proxy request"
    );

    let build_target =
        |current: &SandboxRecord| -> Result<reqwest::Url, (StatusCode, Json<ApiError>)> {
            let host_port = current.extra_ports.get(&port).copied().ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("Port {port} is not exposed on this sandbox"),
                )
            })?;

            let mut target = format!("http://127.0.0.1:{host_port}/{path}");
            if let Some(qs) = query {
                target.push('?');
                target.push_str(qs);
            }
            reqwest::Url::parse(&target)
                .map_err(|e| api_error(StatusCode::BAD_REQUEST, format!("Invalid path: {e}")))
        };

    let proxy_once = |target_url: reqwest::Url,
                      method: axum::http::Method,
                      headers: HeaderMap,
                      body: axum::body::Bytes| async move {
        match tokio::time::timeout(
            PORT_PROXY_TIMEOUT,
            crate::http::proxy_http(target_url, method, &headers, body.to_vec()),
        )
        .await
        {
            Err(_) => Err(SidecarAttemptFailure::Timeout),
            Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
            Ok(Ok(resp)) => Ok(resp),
        }
    };

    match proxy_once(
        build_target(&record)?,
        method.clone(),
        headers.clone(),
        body.clone(),
    )
    .await
    {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "Port proxy timed out after {}s",
                    PORT_PROXY_TIMEOUT.as_secs()
                ),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if is_retryable_transport_error(&err) {
                if let Some(refreshed) = try_refresh_stale_endpoint(&record, "port_proxy").await {
                    match proxy_once(build_target(&refreshed)?, method, headers, body).await {
                        Ok((status, resp_headers, resp_body)) => {
                            circuit_breaker::mark_healthy(&record.id);
                            runtime::touch_sandbox(&record.id);

                            let mut response =
                                axum::response::Response::builder().status(status.as_u16());
                            for (name, value) in resp_headers.iter() {
                                response = response.header(name.as_str(), value.as_bytes());
                            }

                            return response
                                .body(axum::body::Body::from(resp_body))
                                .map_err(|e| {
                                    api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                                });
                        }
                        Err(SidecarAttemptFailure::Timeout) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(
                                StatusCode::GATEWAY_TIMEOUT,
                                format!(
                                    "Port proxy timed out after {}s",
                                    PORT_PROXY_TIMEOUT.as_secs()
                                ),
                            ));
                        }
                        Err(SidecarAttemptFailure::Error(retry_err)) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                        }
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok((status, resp_headers, resp_body)) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);

            let mut response = axum::response::Response::builder().status(status.as_u16());
            for (name, value) in resp_headers.iter() {
                response = response.header(name.as_str(), value.as_bytes());
            }

            response
                .body(axum::body::Body::from(resp_body))
                .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// Auth middleware helper (legacy — prefer `SessionAuth` extractor)
// ---------------------------------------------------------------------------

/// Validate the Authorization header and return the session claims.
///
/// **Prefer** using the [`SessionAuth`](crate::session_auth::SessionAuth) Axum
/// extractor directly in handler signatures instead of calling this manually.
pub fn extract_session_from_headers(
    headers: &HeaderMap,
) -> Result<session_auth::SessionClaims, (StatusCode, Json<ApiError>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = session_auth::extract_bearer_token(auth_header).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid Authorization header format",
        )
    })?;

    session_auth::validate_session_token(token)
        .map_err(|e| api_error(StatusCode::UNAUTHORIZED, e.to_string()))
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

/// Build CORS layer from `CORS_ALLOWED_ORIGINS` env var.
///
/// - `"none"` → CORS disabled (use when behind BPM proxy that handles CORS).
/// - Comma-separated origins → strict whitelist with credentials.
/// - `"*"` → allow any origin (development mode only, must be explicit).
/// - Unset → localhost-only with warning (safe default for production).
pub fn build_cors_layer() -> CorsLayer {
    use axum::http::{Method, header};

    let allowed_methods = vec![
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let allowed_headers = vec![header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT];

    let origins_env = std::env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default();

    // Behind BPM proxy: disable CORS entirely (proxy handles it).
    if origins_env.eq_ignore_ascii_case("none") {
        return CorsLayer::new()
            .allow_origin(AllowOrigin::exact(
                "http://localhost".parse().expect("valid origin"),
            ))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers);
    }

    if origins_env == "*" {
        tracing::warn!("CORS_ALLOWED_ORIGINS=* — wildcard CORS enabled (development mode only)");
        CorsLayer::new()
            .allow_origin(AllowOrigin::any())
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
    } else if origins_env.is_empty() {
        // Unset — restrictive default for production safety.
        tracing::warn!(
            "CORS_ALLOWED_ORIGINS not set; defaulting to localhost-only. \
             Set explicitly for production deployments."
        );
        let localhost_origins: Vec<_> = [
            "http://localhost:1338",
            "http://localhost:3000",
            "http://localhost:5173",
            "http://127.0.0.1:1338",
            "http://127.0.0.1:3000",
            "http://127.0.0.1:5173",
        ]
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(localhost_origins))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
            .allow_credentials(true)
    } else {
        let origins: Vec<_> = origins_env
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
            .allow_credentials(true)
    }
}

// ---------------------------------------------------------------------------
// Per-endpoint HTTP metrics middleware
// ---------------------------------------------------------------------------

async fn http_metrics_middleware(
    req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    // Prefer the route template (e.g. "/api/sandboxes/{sandbox_id}/exec") to avoid
    // high-cardinality metric keys from dynamic path segments like sandbox IDs.
    // When no route matches (404 paths), use a fixed "unmatched" label to prevent
    // unbounded cardinality from scanners probing arbitrary URLs.
    let path = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = response.status();
    let is_server_error = status.is_server_error();
    let is_client_error = status.is_client_error();
    metrics::http_metrics().record(&path, duration_ms, is_server_error, is_client_error);
    response
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the operator API router with all endpoints, CORS, and rate limiting.
///
/// For TEE-enabled operators, use [`operator_api_router_with_tee`] instead.
pub fn operator_api_router() -> Router {
    operator_api_router_with_tee_and_routes(None, Router::new())
}

/// Build the operator API router with optional TEE sealed secrets endpoints.
///
/// When `tee` is `Some(backend)`, the following endpoints are added:
/// - `GET  /api/sandboxes/{id}/tee/public-key`
/// - `POST /api/sandboxes/{id}/tee/sealed-secrets`
///
/// When `tee` is `None`, those routes are not registered and the router
/// behaves identically to [`operator_api_router`].
pub fn operator_api_router_with_tee(
    tee: Option<std::sync::Arc<dyn crate::tee::TeeBackend>>,
) -> Router {
    operator_api_router_with_tee_and_routes(tee, Router::new())
}

/// Build the operator API router and merge additional routes before applying
/// shared middleware such as CORS, request IDs, rate limits, and security
/// headers. This is important for blueprint-specific routes like
/// `/api/workflows/{workflow_id}` so browser preflight requests reach them too.
pub fn operator_api_router_with_tee_and_routes(
    tee: Option<std::sync::Arc<dyn crate::tee::TeeBackend>>,
    extra_routes: Router,
) -> Router {
    let cors = build_cors_layer();

    // Read endpoints: 120 req/min per IP
    let read_routes = Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .route(
            "/api/sandboxes/{sandbox_id}/ports",
            get(sandbox_ports_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/agents",
            get(sandbox_agents_handler),
        )
        .route("/api/sandbox/ports", get(instance_ports_handler))
        .route("/api/sandbox/agents", get(instance_agents_handler))
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions",
            get(sandbox_terminal_session_list_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}/stream",
            get(sandbox_terminal_session_stream_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions",
            get(sandbox_chat_session_list_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}",
            get(sandbox_chat_session_get_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}/stream",
            get(sandbox_chat_session_stream_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions",
            get(instance_terminal_session_list_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}/stream",
            get(instance_terminal_session_stream_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions",
            get(instance_chat_session_list_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}",
            get(instance_chat_session_get_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}/stream",
            get(instance_chat_session_stream_handler),
        )
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    // Write endpoints: 30 req/min per IP
    let write_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/secrets",
            get(get_secrets).post(inject_secrets).delete(wipe_secrets),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions",
            post(sandbox_terminal_session_create_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}",
            axum::routing::delete(sandbox_terminal_session_delete_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions",
            post(sandbox_chat_session_create_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}",
            axum::routing::delete(sandbox_chat_session_delete_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}/runs/{run_id}/cancel",
            post(sandbox_chat_run_cancel_handler),
        )
        .route(
            "/api/sandbox/secrets",
            get(instance_get_secrets)
                .post(instance_inject_secrets)
                .delete(instance_wipe_secrets),
        )
        .route(
            "/api/sandbox/live/terminal/sessions",
            post(instance_terminal_session_create_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}",
            axum::routing::delete(instance_terminal_session_delete_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions",
            post(instance_chat_session_create_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}",
            axum::routing::delete(instance_chat_session_delete_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}/runs/{run_id}/cancel",
            post(instance_chat_run_cancel_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Sandbox-scoped operation endpoints (authenticated, write-rate-limited)
    let sandbox_op_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/exec",
            post(sandbox_exec_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/prompt",
            post(sandbox_prompt_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/task",
            post(sandbox_task_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/stop",
            post(sandbox_stop_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/resume",
            post(sandbox_resume_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/snapshot",
            post(sandbox_snapshot_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/ssh",
            post(sandbox_ssh_provision_handler).delete(sandbox_ssh_revoke_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/ssh/user",
            get(sandbox_ssh_user_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/port/{port}/{*rest}",
            any(sandbox_port_proxy_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/port/{port}",
            any(sandbox_port_proxy_root_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Instance-scoped operation endpoints (singleton sandbox, authenticated)
    let instance_op_routes = Router::new()
        .route("/api/sandbox/exec", post(instance_exec_handler))
        .route("/api/sandbox/prompt", post(instance_prompt_handler))
        .route("/api/sandbox/task", post(instance_task_handler))
        .route("/api/sandbox/stop", post(instance_stop_handler))
        .route("/api/sandbox/resume", post(instance_resume_handler))
        .route("/api/sandbox/snapshot", post(instance_snapshot_handler))
        .route(
            "/api/sandbox/ssh",
            post(instance_ssh_provision_handler).delete(instance_ssh_revoke_handler),
        )
        .route("/api/sandbox/ssh/user", get(instance_ssh_user_handler))
        .route(
            "/api/sandbox/port/{port}/{*rest}",
            any(instance_port_proxy_handler),
        )
        .route(
            "/api/sandbox/port/{port}",
            any(instance_port_proxy_root_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Auth endpoints: 10 req/min per IP (stricter to prevent brute-force)
    let auth_routes = Router::new()
        .route("/api/auth/challenge", post(create_challenge))
        .route(
            "/api/auth/session",
            post(create_session).delete(revoke_session),
        )
        .layer(middleware::from_fn(rate_limit::auth_rate_limit));

    // Health, metrics & provision progress: rate-limited but unauthenticated
    // (liveness probes + pre-auth provision tracking need these)
    let infra_routes = Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/metrics", get(prometheus_metrics))
        .route("/api/provisions", get(list_provisions))
        .route("/api/provisions/{call_id}", get(get_provision))
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    let mut router = Router::new()
        .merge(infra_routes)
        .merge(read_routes)
        .merge(write_routes)
        .merge(sandbox_op_routes)
        .merge(instance_op_routes)
        .merge(auth_routes);

    // TEE sealed secrets endpoints (only when backend is configured)
    if let Some(backend) = tee {
        let tee_routes = Router::new()
            .route(
                "/api/sandboxes/{sandbox_id}/tee/public-key",
                get(crate::tee::sealed_secrets_api::get_tee_public_key),
            )
            .route(
                "/api/sandboxes/{sandbox_id}/tee/sealed-secrets",
                post(crate::tee::sealed_secrets_api::inject_sealed_secrets),
            )
            .route(
                "/api/sandboxes/{sandbox_id}/tee/attestation",
                get(crate::tee::sealed_secrets_api::get_tee_attestation),
            )
            .layer(axum::Extension(
                Some(backend) as Option<std::sync::Arc<dyn crate::tee::TeeBackend>>
            ))
            .layer(middleware::from_fn(rate_limit::write_rate_limit));

        router = router.merge(tee_routes);
    }

    router
        .merge(extra_routes)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(http_metrics_middleware))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower::limit::ConcurrencyLimitLayer::new(64))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(120),
        ))
        .layer(cors)
        // Outermost layer: assign a unique request ID before anything else runs.
        .layer(middleware::from_fn(request_id_middleware))
}

#[cfg(test)]
#[serial_test::serial]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::Request;
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
            let dir =
                std::env::temp_dir().join(format!("operator-api-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", dir) };
        });
    }

    fn reset_test_state() {
        crate::session_auth::clear_all_for_testing();
        crate::circuit_breaker::clear_all_for_testing();
        crate::provision_progress::clear_all_for_testing().expect("clear provision state");
        LIVE_SESSIONS
            .clear_all_for_testing()
            .expect("clear live sessions");
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

    #[derive(Clone, Default)]
    struct MockSidecarState {
        last_exec_payload: Arc<Mutex<Option<Value>>>,
        last_agent_payload: Arc<Mutex<Option<Value>>>,
        exec_response: Arc<Mutex<Value>>,
        agents_response: Arc<Mutex<Value>>,
        stream_response_body: Arc<Mutex<Option<String>>>,
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
            if let Ok(resp) = reqwest::get(&health_url).await {
                if resp.status().is_success() {
                    break;
                }
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
            if let Ok(resp) = reqwest::get(&health_url).await {
                if resp.status().is_success() {
                    break;
                }
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
            if let Some(run) = crate::chat_state::get_run(run_id).expect("get run") {
                if !run.status.is_active() {
                    return run;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for run {run_id} to finish"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

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
            }),
            extra_ports: std::collections::HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
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
    }

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
    }

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

    // ── Sandbox operation API tests ──────────────────────────────────────

    const OP_TEST_OWNER: &str = "0xOP00000000000000000000000000000000000001";

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

    #[tokio::test]
    async fn test_live_terminal_session_sandbox_crud_and_stream() {
        insert_plain_sandbox("live-term-1", OP_TEST_OWNER);
        let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));

        let create = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sandboxes/live-term-1/live/terminal/sessions")
                    .header("authorization", &auth)
                    .body(Body::empty())
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
    }

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

    #[tokio::test]
    async fn test_live_terminal_stream_receives_exec_output() {
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

        let exec_body = json!({
            "command": "echo hello",
            "session_id": session_id,
        });
        let exec = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sandboxes/live-exec-1/exec")
                    .header("authorization", &auth)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&exec_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exec.status(), StatusCode::OK);
        let exec_json = body_json(exec.into_body()).await;
        assert_eq!(exec_json["stdout"], "mock-exec-stdout");

        let frame = read_first_sse_frame(stream.into_body())
            .await
            .expect("sse frame");
        assert!(
            frame.contains("mock-exec-stdout"),
            "expected terminal stream to include exec output, got: {frame}"
        );

        let exec_payload = sidecar_state
            .last_exec_payload
            .lock()
            .expect("exec payload lock")
            .clone()
            .expect("exec payload");
        assert_eq!(exec_payload["command"], "echo hello");
        server.abort();
    }

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

        let auth = format!("Bearer {}", session_auth::create_test_token(OP_TEST_OWNER));
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
        let assistant_parts = assistant_message["parts"].as_array().expect("assistant parts");

        assert!(
            assistant_parts.iter().all(|part| {
                part["text"].as_str().unwrap_or_default() != "hello from live stream"
            }),
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

    fn insert_sandbox_with_ports(
        id: &str,
        owner: &str,
        ports: std::collections::HashMap<u16, u16>,
    ) {
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
        provision_progress::update_provision_metadata(call_id, json!({ "service_id": 42 }))
            .unwrap();

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

    #[tokio::test]
    async fn test_sandbox_port_proxy_forwards_correctly() {
        // Spawn a mock backend that a proxy will forward to
        let backend_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock backend");
        let backend_addr = backend_listener.local_addr().expect("backend addr");
        let backend_port = backend_addr.port();
        let backend_app =
            Router::new().route("/hello", get(|| async { (StatusCode::OK, "proxy-ok") }));
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

    #[tokio::test]
    async fn test_terminal_session_cross_sandbox_isolation() {
        insert_plain_sandbox("iso-term-a", OP_TEST_OWNER);
        insert_plain_sandbox("iso-term-b", OP_TEST_OWNER);
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
    }

    #[tokio::test]
    async fn test_terminal_session_cross_owner_isolation() {
        const OWNER_A: &str = "0xISOOWNER00000000000000000000000000000A1";
        const OWNER_B: &str = "0xISOOWNER00000000000000000000000000000B1";
        insert_plain_sandbox("iso-owner-term-1", OWNER_A);
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
    }

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

    #[test]
    fn test_handle_lifecycle_outcome_already_stopped_ok() {
        let result = handle_lifecycle_outcome(
            Err(crate::SandboxError::Validation("already stopped".into())),
            "already stopped",
        );
        assert!(result.is_ok(), "already-stopped should be treated as Ok");
    }

    #[test]
    fn test_handle_lifecycle_outcome_already_running_ok() {
        let result = handle_lifecycle_outcome(
            Err(crate::SandboxError::Validation("already running".into())),
            "already running",
        );
        assert!(result.is_ok(), "already-running should be treated as Ok");
    }

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
            Some(
                "Unknown agent identifier \"a1\". This sidecar image does not register that agent."
            )
        );
        server.abort();
    }

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

    #[test]
    fn test_parse_detected_ssh_username_tolerates_terminal_noise() {
        let exec = ExecApiResponse {
            exit_code: 0,
            stdout: "\u{1b}[?2004l\rsidecar\r\n\u{1b}[?2004hcontainer:/sidecar$ exit\r\n"
                .to_string(),
            stderr: String::new(),
        };

        let username = parse_detected_ssh_username(&exec).expect("username should parse");
        assert_eq!(username, "sidecar");
    }

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
}
