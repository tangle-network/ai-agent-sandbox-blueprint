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
use crate::workflows::run_task_request;

/// Extract exec response fields, handling both flat and nested `data` shapes.
fn extract_exec_fields(parsed: &Value) -> (u32, String, String) {
    let exit_code = parsed
        .get("exitCode")
        .and_then(Value::as_u64)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|d| d.get("exitCode"))
                .and_then(Value::as_u64)
        })
        .unwrap_or(0) as u32;

    let stdout = parsed
        .get("stdout")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|d| d.get("stdout"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();

    let stderr = parsed
        .get("stderr")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|d| d.get("stderr"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();

    (exit_code, stdout, stderr)
}

pub async fn sandbox_exec(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxExecRequest>,
) -> Result<TangleResult<SandboxExecResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut payload = Map::new();
    payload.insert(
        "command".to_string(),
        Value::String(request.command.to_string()),
    );
    if !request.cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(request.cwd.to_string()));
    }
    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }
    if !request.env_json.trim().is_empty() {
        let env_map = crate::util::parse_json_object(&request.env_json, "env_json")?;
        if let Some(env_map) = env_map {
            payload.insert("env".to_string(), env_map);
        }
    }

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        &token,
        Value::Object(payload),
    )
    .await?;

    let (exit_code, stdout, stderr) = extract_exec_fields(&parsed);

    Ok(TangleResult(SandboxExecResponse {
        exit_code,
        stdout,
        stderr,
    }))
}

pub async fn sandbox_prompt(
    Caller(_caller): Caller,
    TangleArg(request): TangleArg<SandboxPromptRequest>,
) -> Result<TangleResult<SandboxPromptResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut payload = Map::new();
    payload.insert(
        "identifier".to_string(),
        Value::String("default-agent".to_string()),
    );
    payload.insert(
        "message".to_string(),
        Value::String(request.message.to_string()),
    );

    if !request.session_id.is_empty() {
        payload.insert(
            "sessionId".to_string(),
            Value::String(request.session_id.to_string()),
        );
    }

    if !request.model.is_empty() {
        payload.insert("backend".to_string(), json!({ "model": request.model }));
    }

    if !request.context_json.trim().is_empty() {
        let context = crate::util::parse_json_object(&request.context_json, "context_json")?;
        if let Some(context) = context {
            payload.insert("metadata".to_string(), context);
        }
    }

    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }

    let m = crate::metrics::metrics();
    let _session = m.session_guard();

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/agents/run",
        &token,
        Value::Object(payload),
    )
    .await?;

    let (success, response, error, trace_id) = crate::extract_agent_fields(&parsed);

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

    if success {
        m.record_job(duration_ms, input_tokens, output_tokens);
    } else {
        m.record_failure();
    }

    Ok(TangleResult(SandboxPromptResponse {
        success,
        response,
        error,
        trace_id,
        duration_ms,
        input_tokens,
        output_tokens,
    }))
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
