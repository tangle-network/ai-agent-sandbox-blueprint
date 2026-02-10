use serde_json::{Map, Value, json};

use crate::SandboxExecRequest;
use crate::SandboxExecResponse;
use crate::SandboxPromptRequest;
use crate::SandboxPromptResponse;
use crate::SandboxTaskRequest;
use crate::SandboxTaskResponse;
use crate::auth::require_sidecar_token;
use crate::http::sidecar_post_json;
use crate::runtime::require_sidecar_auth;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};

// ---------------------------------------------------------------------------
// Exec (terminal commands)
// ---------------------------------------------------------------------------

/// Extract exec response fields from the sidecar `/terminals/commands` response.
///
/// Response shape: `{ success, result: { exitCode, stdout, stderr, duration } }`
pub fn extract_exec_fields(parsed: &Value) -> (u32, String, String) {
    let result = parsed.get("result");

    let exit_code = result
        .and_then(|r| r.get("exitCode"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;

    let stdout = result
        .and_then(|r| r.get("stdout"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let stderr = result
        .and_then(|r| r.get("stderr"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    (exit_code, stdout, stderr)
}

/// Build the JSON payload for `/terminals/commands`.
pub fn build_exec_payload(
    command: &str,
    cwd: &str,
    env_json: &str,
    timeout_ms: u64,
) -> Map<String, Value> {
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
    payload
}

/// Run an exec request against a sidecar. Callable from tests without Tangle extractors.
pub async fn run_exec_request(
    request: &SandboxExecRequest,
) -> Result<SandboxExecResponse, String> {
    let payload = build_exec_payload(
        &request.command,
        &request.cwd,
        &request.env_json,
        request.timeout_ms,
    );

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/terminals/commands",
        &request.sidecar_token,
        Value::Object(payload),
    )
    .await
    .map_err(|e| e.to_string())?;

    let (exit_code, stdout, stderr) = extract_exec_fields(&parsed);

    Ok(SandboxExecResponse {
        exit_code,
        stdout,
        stderr,
    })
}

pub async fn sandbox_exec(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxExecRequest>,
) -> Result<TangleResult<SandboxExecResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut request = request;
    request.sidecar_token = token;
    let response = run_exec_request(&request).await?;
    Ok(TangleResult(response))
}

// ---------------------------------------------------------------------------
// Agent (prompt / task) — shared payload builder
// ---------------------------------------------------------------------------

/// Build the common `/agents/run` payload used by both prompt and task requests.
fn build_agent_payload(
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    extra_metadata: Option<Map<String, Value>>,
) -> Result<Map<String, Value>, String> {
    let mut payload = Map::new();
    payload.insert(
        "identifier".to_string(),
        Value::String("default".to_string()),
    );
    payload.insert(
        "message".to_string(),
        Value::String(message.to_string()),
    );

    if !session_id.is_empty() {
        payload.insert(
            "sessionId".to_string(),
            Value::String(session_id.to_string()),
        );
    }

    if !model.is_empty() {
        payload.insert("backend".to_string(), json!({ "model": model }));
    }

    let mut metadata = Map::new();
    if !context_json.trim().is_empty() {
        let context = crate::util::parse_json_object(context_json, "context_json")?;
        if let Some(Value::Object(ctx)) = context {
            metadata.extend(ctx);
        }
    }

    if let Some(extra) = extra_metadata {
        metadata.extend(extra);
    }

    if !metadata.is_empty() {
        payload.insert("metadata".to_string(), Value::Object(metadata));
    }

    if timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(timeout_ms));
    }

    Ok(payload)
}

/// Parse the common agent response fields from the sidecar JSON.
struct AgentResponse {
    success: bool,
    response: String,
    error: String,
    trace_id: String,
    duration_ms: u64,
    input_tokens: u32,
    output_tokens: u32,
    session_id: String,
}

fn parse_agent_response(parsed: &Value, fallback_session_id: &str) -> AgentResponse {
    let (success, response, error, trace_id) = crate::extract_agent_fields(parsed);

    let duration_ms = parsed
        .get("durationMs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input_tokens = parsed
        .get("usage")
        .and_then(|u| u.get("inputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let output_tokens = parsed
        .get("usage")
        .and_then(|u| u.get("outputTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let session_id = parsed
        .get("sessionId")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|d| d.get("metadata"))
                .and_then(|m| m.get("sessionId"))
                .and_then(Value::as_str)
        })
        .unwrap_or(fallback_session_id)
        .to_string();

    AgentResponse {
        success,
        response,
        error,
        trace_id,
        duration_ms,
        input_tokens,
        output_tokens,
        session_id,
    }
}

/// Send payload to `/agents/run`, parse response, record metrics.
async fn call_agent(
    sidecar_url: &str,
    sidecar_token: &str,
    payload: Map<String, Value>,
    fallback_session_id: &str,
) -> Result<AgentResponse, String> {
    let m = crate::metrics::metrics();
    let _session = m.session_guard();

    let parsed = sidecar_post_json(
        sidecar_url,
        "/agents/run",
        sidecar_token,
        Value::Object(payload),
    )
    .await
    .map_err(|e| e.to_string())?;

    let resp = parse_agent_response(&parsed, fallback_session_id);

    if resp.success {
        m.record_job(resp.duration_ms, resp.input_tokens, resp.output_tokens);
    } else {
        m.record_failure();
    }

    Ok(resp)
}

// ---------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------

/// Run a prompt request against a sidecar. Callable from tests.
pub async fn run_prompt_request(
    request: &SandboxPromptRequest,
) -> Result<SandboxPromptResponse, String> {
    let payload = build_agent_payload(
        &request.message,
        &request.session_id,
        &request.model,
        &request.context_json,
        request.timeout_ms,
        None,
    )?;

    let resp = call_agent(
        &request.sidecar_url,
        &request.sidecar_token,
        payload,
        &request.session_id,
    )
    .await?;

    Ok(SandboxPromptResponse {
        success: resp.success,
        response: resp.response,
        error: resp.error,
        trace_id: resp.trace_id,
        duration_ms: resp.duration_ms,
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
    })
}

pub async fn sandbox_prompt(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxPromptRequest>,
) -> Result<TangleResult<SandboxPromptResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut request = request;
    request.sidecar_token = token;
    let response = run_prompt_request(&request).await?;
    Ok(TangleResult(response))
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// Run a task request against a sidecar. Callable from tests.
pub async fn run_task_request(
    request: &SandboxTaskRequest,
) -> Result<SandboxTaskResponse, String> {
    let mut extra = Map::new();
    if request.max_turns > 0 {
        extra.insert("maxTurns".to_string(), json!(request.max_turns));
        extra.insert("maxSteps".to_string(), json!(request.max_turns));
    }

    let payload = build_agent_payload(
        &request.prompt,
        &request.session_id,
        &request.model,
        &request.context_json,
        request.timeout_ms,
        if extra.is_empty() { None } else { Some(extra) },
    )?;

    let resp = call_agent(
        &request.sidecar_url,
        &request.sidecar_token,
        payload,
        &request.session_id,
    )
    .await?;

    Ok(SandboxTaskResponse {
        success: resp.success,
        result: resp.response,
        error: resp.error,
        trace_id: resp.trace_id,
        duration_ms: resp.duration_ms,
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
        session_id: resp.session_id,
    })
}

pub async fn sandbox_task(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxTaskRequest>,
) -> Result<TangleResult<SandboxTaskResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut request = request;
    request.sidecar_token = token;

    let response = run_task_request(&request).await?;
    Ok(TangleResult(response))
}
