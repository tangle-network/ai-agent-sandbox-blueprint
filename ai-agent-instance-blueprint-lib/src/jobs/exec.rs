use serde_json::{Map, Value, json};

use crate::InstanceExecRequest;
use crate::InstanceExecResponse;
use crate::InstancePromptRequest;
use crate::InstancePromptResponse;
use crate::InstanceTaskRequest;
use crate::InstanceTaskResponse;
use crate::http::sidecar_post_json;
use crate::require_instance_sandbox;
use crate::tangle::extract::{Caller, TangleArg, TangleResult};

// ─────────────────────────────────────────────────────────────────────────────
// Exec
// ─────────────────────────────────────────────────────────────────────────────

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

/// Core exec logic — testable without TangleArg extractors.
pub async fn run_instance_exec(
    sidecar_url: &str,
    sidecar_token: &str,
    sandbox_id: &str,
    request: &InstanceExecRequest,
) -> Result<InstanceExecResponse, String> {
    let payload = build_exec_payload(
        &request.command,
        &request.cwd,
        &request.env_json,
        request.timeout_ms,
    );

    let parsed = sidecar_post_json(
        sidecar_url,
        "/terminals/commands",
        sidecar_token,
        Value::Object(payload),
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::runtime::touch_sandbox(sandbox_id);

    let (exit_code, stdout, stderr) = extract_exec_fields(&parsed);

    Ok(InstanceExecResponse {
        exit_code,
        stdout,
        stderr,
    })
}

pub async fn instance_exec(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceExecRequest>,
) -> Result<TangleResult<InstanceExecResponse>, String> {
    let sandbox = require_instance_sandbox()?;
    let resp =
        run_instance_exec(&sandbox.sidecar_url, &sandbox.token, &sandbox.id, &request).await?;
    Ok(TangleResult(resp))
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent (prompt / task) — shared helpers
// ─────────────────────────────────────────────────────────────────────────────

pub fn build_agent_payload(
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
    payload.insert("message".to_string(), Value::String(message.to_string()));

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
        let context = crate::util::parse_json_object(context_json, "context_json")
            .map_err(|e| e.to_string())?;
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

pub struct AgentResponse {
    pub success: bool,
    pub response: String,
    pub error: String,
    pub trace_id: String,
    pub duration_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub session_id: String,
}

pub fn parse_agent_response(parsed: &Value, fallback_session_id: &str) -> AgentResponse {
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

pub async fn call_agent(
    sidecar_url: &str,
    sidecar_token: &str,
    sandbox_id: &str,
    payload: Map<String, Value>,
    fallback_session_id: &str,
) -> Result<AgentResponse, String> {
    crate::runtime::touch_sandbox(sandbox_id);

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

// ─────────────────────────────────────────────────────────────────────────────
// Prompt
// ─────────────────────────────────────────────────────────────────────────────

/// Core prompt logic — testable without TangleArg extractors.
pub async fn run_instance_prompt(
    sidecar_url: &str,
    sidecar_token: &str,
    sandbox_id: &str,
    request: &InstancePromptRequest,
) -> Result<InstancePromptResponse, String> {
    let payload = build_agent_payload(
        &request.message,
        &request.session_id,
        &request.model,
        &request.context_json,
        request.timeout_ms,
        None,
    )?;

    let resp = call_agent(
        sidecar_url,
        sidecar_token,
        sandbox_id,
        payload,
        &request.session_id,
    )
    .await?;

    Ok(InstancePromptResponse {
        success: resp.success,
        response: resp.response,
        error: resp.error,
        trace_id: resp.trace_id,
        duration_ms: resp.duration_ms,
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
    })
}

pub async fn instance_prompt(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstancePromptRequest>,
) -> Result<TangleResult<InstancePromptResponse>, String> {
    let sandbox = require_instance_sandbox()?;
    let resp =
        run_instance_prompt(&sandbox.sidecar_url, &sandbox.token, &sandbox.id, &request).await?;
    Ok(TangleResult(resp))
}

// ─────────────────────────────────────────────────────────────────────────────
// Task
// ─────────────────────────────────────────────────────────────────────────────

/// Core task logic — testable without TangleArg extractors.
pub async fn run_instance_task(
    sidecar_url: &str,
    sidecar_token: &str,
    sandbox_id: &str,
    request: &InstanceTaskRequest,
) -> Result<InstanceTaskResponse, String> {
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
        sidecar_url,
        sidecar_token,
        sandbox_id,
        payload,
        &request.session_id,
    )
    .await?;

    Ok(InstanceTaskResponse {
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

pub async fn instance_task(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<InstanceTaskRequest>,
) -> Result<TangleResult<InstanceTaskResponse>, String> {
    let sandbox = require_instance_sandbox()?;
    let resp =
        run_instance_task(&sandbox.sidecar_url, &sandbox.token, &sandbox.id, &request).await?;
    Ok(TangleResult(resp))
}
