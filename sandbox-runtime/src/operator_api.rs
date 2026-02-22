//! Axum-based operator API for sandbox management.
//!
//! Provides REST endpoints for:
//! - Listing active sandboxes
//! - Querying provision progress
//! - Session auth (challenge/response + PASETO tokens)
//! - Sandbox operations (exec, prompt, task, stop, resume, snapshot, SSH)

use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::api_types::*;
use crate::http::sidecar_post_json;
use crate::metrics;
use crate::provision_progress;
use crate::rate_limit;
use crate::runtime::{self, SandboxRecord, SandboxState, sandboxes};
use crate::secret_provisioning;
use crate::session_auth::{self, SessionAuth};

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiError {
    error: String,
}

pub(crate) fn api_error(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: msg.into() }))
}

// ---------------------------------------------------------------------------
// Sandbox endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SandboxSummary {
    id: String,
    sidecar_url: String,
    state: String,
    cpu_cores: u64,
    memory_mb: u64,
    created_at: u64,
    last_activity_at: u64,
}

impl From<&SandboxRecord> for SandboxSummary {
    fn from(r: &SandboxRecord) -> Self {
        Self {
            id: r.id.clone(),
            sidecar_url: r.sidecar_url.clone(),
            state: match r.state {
                SandboxState::Running => "running".into(),
                SandboxState::Stopped => "stopped".into(),
            },
            cpu_cores: r.cpu_cores,
            memory_mb: r.memory_mb,
            created_at: r.created_at,
            last_activity_at: r.last_activity_at,
        }
    }
}

async fn list_sandboxes(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    match sandboxes().and_then(|s| s.values()) {
        Ok(records) => {
            let summaries: Vec<SandboxSummary> = records
                .iter()
                .filter(|r| r.owner.is_empty() || r.owner.eq_ignore_ascii_case(&address))
                .map(SandboxSummary::from)
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "sandboxes": summaries }))).into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Provision progress endpoints
// ---------------------------------------------------------------------------

async fn get_provision(
    Path(call_id): Path<u64>,
) -> impl IntoResponse {
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
        Ok(provisions) => (StatusCode::OK, Json(serde_json::json!({ "provisions": provisions }))).into_response(),
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
    let challenge = session_auth::create_challenge();
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
        Err(e) => api_error(StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
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
}

async fn inject_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    match secret_provisioning::inject_secrets(&sandbox_id, body.env_json, None).await {
        Ok(record) => (
            StatusCode::OK,
            Json(SecretsResponse {
                status: "secrets_configured".to_string(),
                sandbox_id: record.id,
            }),
        )
            .into_response(),
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
        Ok(record) => (
            StatusCode::OK,
            Json(SecretsResponse {
                status: "secrets_wiped".to_string(),
                sandbox_id: record.id,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Health & metrics endpoints (unauthenticated)
// ---------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok" })),
    )
}

async fn prometheus_metrics() -> impl IntoResponse {
    let body = metrics::metrics().render_prometheus();
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
fn resolve_sandbox(sandbox_id: &str, caller: &str) -> Result<SandboxRecord, (StatusCode, Json<ApiError>)> {
    runtime::require_sandbox_owner(sandbox_id, caller)
        .map_err(|e| {
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

    if !record.owner.is_empty() && !record.owner.eq_ignore_ascii_case(caller) {
        return Err(api_error(StatusCode::FORBIDDEN, "Not authorized for this instance"));
    }
    Ok(record)
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

/// Build `/agents/run` payload for prompt/task operations.
fn build_agent_payload(
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    max_turns: Option<u64>,
) -> Value {
    let mut payload = Map::new();
    payload.insert("identifier".into(), json!("default"));
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
                if let Ok(Some(Value::Object(ctx))) = crate::util::parse_json_object(context_json, "context_json") {
                    metadata.extend(ctx);
                }
            }
            payload.insert("metadata".into(), Value::Object(metadata));
        }
    } else if !context_json.trim().is_empty() {
        if let Ok(Some(Value::Object(ctx))) = crate::util::parse_json_object(context_json, "context_json") {
            payload.insert("metadata".into(), Value::Object(ctx));
        }
    }

    if timeout_ms > 0 {
        payload.insert("timeout".into(), json!(timeout_ms));
    }
    Value::Object(payload)
}

/// Parse agent response from sidecar (used by both prompt and task).
fn parse_agent_response(parsed: &Value) -> (bool, String, String, String, String) {
    let success = parsed.get("success").and_then(Value::as_bool).unwrap_or(false);
    let response = parsed
        .get("response")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("data").and_then(|d| d.get("finalText")).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let error = parsed
        .get("error")
        .and_then(|e| e.get("message").and_then(Value::as_str).or_else(|| e.as_str()))
        .unwrap_or_default()
        .to_string();
    let trace_id = parsed.get("traceId").and_then(Value::as_str).unwrap_or_default().to_string();
    let session_id = parsed
        .get("sessionId")
        .or_else(|| parsed.get("data").and_then(|d| d.get("metadata")).and_then(|m| m.get("sessionId")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    (success, response, error, trace_id, session_id)
}

/// Execute a sidecar operation and return the result, touching the sandbox activity.
async fn exec_on_sidecar(record: &SandboxRecord, req: &ExecApiRequest) -> Result<ExecApiResponse, (StatusCode, Json<ApiError>)> {
    let payload = build_exec_payload(&req.command, &req.cwd, &req.env_json, req.timeout_ms);
    let parsed = sidecar_post_json(&record.sidecar_url, "/terminals/commands", &record.token, payload)
        .await
        .map_err(|e| api_error(StatusCode::BAD_GATEWAY, e.to_string()))?;
    runtime::touch_sandbox(&record.id);
    Ok(parse_exec_response(&parsed))
}

/// Run a prompt/task on the sidecar agent.
async fn agent_on_sidecar(
    record: &SandboxRecord,
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    max_turns: Option<u64>,
) -> Result<(bool, String, String, String, String), (StatusCode, Json<ApiError>)> {
    let payload = build_agent_payload(message, session_id, model, context_json, timeout_ms, max_turns);
    let parsed = sidecar_post_json(&record.sidecar_url, "/agents/run", &record.token, payload)
        .await
        .map_err(|e| api_error(StatusCode::BAD_GATEWAY, e.to_string()))?;
    runtime::touch_sandbox(&record.id);
    Ok(parse_agent_response(&parsed))
}

// ── Exec ─────────────────────────────────────────────────────────────────

async fn sandbox_exec_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = exec_on_sidecar(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_exec_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = exec_on_sidecar(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ── Prompt ───────────────────────────────────────────────────────────────

async fn sandbox_prompt_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let (success, response, error, trace_id, _) = agent_on_sidecar(
        &record, &req.message, &req.session_id, &req.model, &req.context_json, req.timeout_ms, None,
    ).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(PromptApiResponse {
        success, response, error, trace_id, duration_ms: 0, input_tokens: 0, output_tokens: 0,
    })))
}

async fn instance_prompt_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let (success, response, error, trace_id, _) = agent_on_sidecar(
        &record, &req.message, &req.session_id, &req.model, &req.context_json, req.timeout_ms, None,
    ).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(PromptApiResponse {
        success, response, error, trace_id, duration_ms: 0, input_tokens: 0, output_tokens: 0,
    })))
}

// ── Task ─────────────────────────────────────────────────────────────────

async fn sandbox_task_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let (success, result, error, trace_id, session_id) = agent_on_sidecar(
        &record, &req.prompt, &req.session_id, &req.model, &req.context_json, req.timeout_ms, Some(req.max_turns),
    ).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(TaskApiResponse {
        success, result, error, trace_id, session_id, duration_ms: 0, input_tokens: 0, output_tokens: 0,
    })))
}

async fn instance_task_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let (success, result, error, trace_id, session_id) = agent_on_sidecar(
        &record, &req.prompt, &req.session_id, &req.model, &req.context_json, req.timeout_ms, Some(req.max_turns),
    ).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(TaskApiResponse {
        success, result, error, trace_id, session_id, duration_ms: 0, input_tokens: 0, output_tokens: 0,
    })))
}

// ── Stop / Resume ────────────────────────────────────────────────────────

async fn sandbox_stop_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    runtime::stop_sidecar(&record)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(LifecycleApiResponse {
        success: true, sandbox_id: record.id, state: "stopped".into(),
    })))
}

async fn sandbox_resume_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    runtime::resume_sidecar(&record)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(LifecycleApiResponse {
        success: true, sandbox_id: record.id, state: "running".into(),
    })))
}

async fn instance_stop_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    runtime::stop_sidecar(&record)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(LifecycleApiResponse {
        success: true, sandbox_id: id, state: "stopped".into(),
    })))
}

async fn instance_resume_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let id = record.id.clone();
    runtime::resume_sidecar(&record)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(LifecycleApiResponse {
        success: true, sandbox_id: id, state: "running".into(),
    })))
}

// ── Snapshot ─────────────────────────────────────────────────────────────

async fn run_snapshot(record: &SandboxRecord, req: &SnapshotApiRequest) -> Result<SnapshotApiResponse, (StatusCode, Json<ApiError>)> {
    if req.destination.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "Snapshot destination is required"));
    }
    let command = crate::util::build_snapshot_command(&req.destination, req.include_workspace, req.include_state)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let payload = json!({ "command": format!("sh -c {}", crate::util::shell_escape(&command)) });
    let result = sidecar_post_json(&record.sidecar_url, "/terminals/commands", &record.token, payload)
        .await
        .map_err(|e| api_error(StatusCode::BAD_GATEWAY, e.to_string()))?;
    runtime::touch_sandbox(&record.id);
    Ok(SnapshotApiResponse { success: true, result })
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

fn build_ssh_provision_command(username: &str, public_key: &str) -> String {
    let user_arg = crate::util::shell_escape(username);
    let key_arg = crate::util::shell_escape(public_key);
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
if ! grep -qxF {key_arg} \"$home/.ssh/authorized_keys\" 2>/dev/null; then \
    echo {key_arg} >> \"$home/.ssh/authorized_keys\"; \
fi; chmod 600 \"$home/.ssh/authorized_keys\""
    )
}

fn build_ssh_revoke_cmd(username: &str, public_key: &str) -> String {
    let user_arg = crate::util::shell_escape(username);
    let key_arg = crate::util::shell_escape(public_key);
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
    tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
    grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
    mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    )
}

async fn run_ssh_provision(record: &SandboxRecord, req: &SshProvisionApiRequest) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let username = crate::util::normalize_username(&req.username)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let command = build_ssh_provision_command(&username, &req.public_key);
    let payload = json!({ "command": format!("sh -c {}", crate::util::shell_escape(&command)) });
    let result = sidecar_post_json(&record.sidecar_url, "/terminals/commands", &record.token, payload)
        .await
        .map_err(|e| api_error(StatusCode::BAD_GATEWAY, e.to_string()))?;
    runtime::touch_sandbox(&record.id);
    Ok(SshApiResponse { success: true, result })
}

async fn run_ssh_revoke(record: &SandboxRecord, req: &SshRevokeApiRequest) -> Result<SshApiResponse, (StatusCode, Json<ApiError>)> {
    let username = crate::util::normalize_username(&req.username)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e.to_string()))?;
    let command = build_ssh_revoke_cmd(&username, &req.public_key);
    let payload = json!({ "command": format!("sh -c {}", crate::util::shell_escape(&command)) });
    let result = sidecar_post_json(&record.sidecar_url, "/terminals/commands", &record.token, payload)
        .await
        .map_err(|e| api_error(StatusCode::BAD_GATEWAY, e.to_string()))?;
    runtime::touch_sandbox(&record.id);
    Ok(SshApiResponse { success: true, result })
}

async fn sandbox_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn sandbox_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_ssh_provision_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshProvisionApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = run_ssh_provision(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

async fn instance_ssh_revoke_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<SshRevokeApiRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = run_ssh_revoke(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

// ---------------------------------------------------------------------------
// Auth middleware helper (legacy — prefer `SessionAuth` extractor)
// ---------------------------------------------------------------------------

/// Validate the Authorization header and return the session claims.
///
/// **Prefer** using the [`SessionAuth`](crate::session_auth::SessionAuth) Axum
/// extractor directly in handler signatures instead of calling this manually.
pub fn extract_session_from_headers(headers: &HeaderMap) -> Result<session_auth::SessionClaims, (StatusCode, Json<ApiError>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = session_auth::extract_bearer_token(auth_header)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Invalid Authorization header format"))?;

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
/// - Unset or `"*"` → allow any origin (development mode only).
pub fn build_cors_layer() -> CorsLayer {
    use axum::http::{header, Method};

    let allowed_methods = vec![
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let allowed_headers = vec![
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
        header::ACCEPT,
    ];

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

    if origins_env.is_empty() || origins_env == "*" {
        CorsLayer::new()
            .allow_origin(AllowOrigin::any())
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
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
// Router builder
// ---------------------------------------------------------------------------

/// Build the operator API router with all endpoints, CORS, and rate limiting.
///
/// For TEE-enabled operators, use [`operator_api_router_with_tee`] instead.
pub fn operator_api_router() -> Router {
    operator_api_router_with_tee(None)
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
    let cors = build_cors_layer();

    // Read endpoints: 120 req/min per IP
    let read_routes = Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    // Write endpoints: 30 req/min per IP
    let write_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/secrets",
            post(inject_secrets).delete(wipe_secrets),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Sandbox-scoped operation endpoints (authenticated, write-rate-limited)
    let sandbox_op_routes = Router::new()
        .route("/api/sandboxes/{sandbox_id}/exec", post(sandbox_exec_handler))
        .route("/api/sandboxes/{sandbox_id}/prompt", post(sandbox_prompt_handler))
        .route("/api/sandboxes/{sandbox_id}/task", post(sandbox_task_handler))
        .route("/api/sandboxes/{sandbox_id}/stop", post(sandbox_stop_handler))
        .route("/api/sandboxes/{sandbox_id}/resume", post(sandbox_resume_handler))
        .route("/api/sandboxes/{sandbox_id}/snapshot", post(sandbox_snapshot_handler))
        .route("/api/sandboxes/{sandbox_id}/ssh", post(sandbox_ssh_provision_handler).delete(sandbox_ssh_revoke_handler))
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Instance-scoped operation endpoints (singleton sandbox, authenticated)
    let instance_op_routes = Router::new()
        .route("/api/sandbox/exec", post(instance_exec_handler))
        .route("/api/sandbox/prompt", post(instance_prompt_handler))
        .route("/api/sandbox/task", post(instance_task_handler))
        .route("/api/sandbox/stop", post(instance_stop_handler))
        .route("/api/sandbox/resume", post(instance_resume_handler))
        .route("/api/sandbox/snapshot", post(instance_snapshot_handler))
        .route("/api/sandbox/ssh", post(instance_ssh_provision_handler).delete(instance_ssh_revoke_handler))
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Auth endpoints: 10 req/min per IP (stricter to prevent brute-force)
    let auth_routes = Router::new()
        .route("/api/auth/challenge", post(create_challenge))
        .route("/api/auth/session", post(create_session))
        .layer(middleware::from_fn(rate_limit::auth_rate_limit));

    // Health, metrics & provision progress: rate-limited but unauthenticated
    // (liveness probes + pre-auth provision tracking need these)
    let infra_routes = Router::new()
        .route("/health", get(health))
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
            .layer(axum::Extension(Some(backend) as Option<std::sync::Arc<dyn crate::tee::TeeBackend>>))
            .layer(middleware::from_fn(rate_limit::write_rate_limit));

        router = router.merge(tee_routes);
    }

    router.layer(cors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    use std::sync::Once;
    static INIT: Once = Once::new();
    fn init() {
        INIT.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("operator-api-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", dir) };
        });
    }

    fn app() -> Router {
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

    #[tokio::test]
    async fn test_list_sandboxes_empty() {
        init();

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

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["status"], "ok");
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
                    .header("origin", "http://localhost:5173")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("access-control-allow-origin"));
    }

    // ── TEE sealed secrets API tests ──────────────────────────────────────

    fn tee_app() -> Router {
        let mock = std::sync::Arc::new(
            crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx),
        );
        operator_api_router_with_tee(Some(mock))
    }

    /// Insert a sandbox record with TEE fields into the store.
    fn insert_tee_sandbox(id: &str, deployment_id: &str, owner: &str) {
        init();
        use crate::runtime::{SandboxRecord, SandboxState, sandboxes};
        sandboxes()
            .unwrap()
            .insert(
                id.to_string(),
                SandboxRecord {
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
                    name: "tee-sandbox".into(),
                    agent_identifier: String::new(),
                    metadata_json: "{}".into(),
                    disk_gb: 50,
                    stack: String::new(),
                    owner: owner.to_string(),
                    tee_config: Some(crate::tee::TeeConfig {
                        required: true,
                        tee_type: crate::tee::TeeType::Tdx,
                    }),
                },
            )
            .unwrap();
    }

    /// Insert a non-TEE sandbox into the store.
    fn insert_plain_sandbox(id: &str, owner: &str) {
        init();
        use crate::runtime::{SandboxRecord, SandboxState, sandboxes};
        sandboxes()
            .unwrap()
            .insert(
                id.to_string(),
                SandboxRecord {
                    id: id.to_string(),
                    container_id: format!("ctr-{id}"),
                    sidecar_url: "http://localhost:9999".into(),
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
                    name: "plain-sandbox".into(),
                    agent_identifier: String::new(),
                    metadata_json: "{}".into(),
                    disk_gb: 10,
                    stack: String::new(),
                    owner: owner.to_string(),
                    tee_config: None,
                },
            )
            .unwrap();
    }

    // Use a distinct owner for TEE tests so sandbox inserts don't pollute
    // the test_list_sandboxes_empty assertion (which uses a different address).
    const TEE_TEST_OWNER: &str = "0xTEE0000000000000000000000000000000000001";

    #[tokio::test]
    async fn test_tee_public_key_success() {
        insert_tee_sandbox("tee-pk-1", "deploy-pk-1", TEE_TEST_OWNER);
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

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
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

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
            session_auth::create_test_token(
                "0x1234567890abcdef1234567890abcdef12345678"
            )
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
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

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
            session_auth::create_test_token(
                "0x1234567890abcdef1234567890abcdef12345678"
            )
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
    }
}
