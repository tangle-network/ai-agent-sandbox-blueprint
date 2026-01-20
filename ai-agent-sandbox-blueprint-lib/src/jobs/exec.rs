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
use crate::tangle_evm::extract::{Caller, TangleEvmArg, TangleEvmResult};
use crate::workflows::run_task_request;

pub async fn sandbox_exec(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxExecRequest>,
) -> Result<TangleEvmResult<SandboxExecResponse>, String> {
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
        crate::runtime::SidecarRuntimeConfig::load().timeout,
    )
    .await?;

    Ok(TangleEvmResult(SandboxExecResponse {
        exit_code: parsed
            .get("exitCode")
            .and_then(Value::as_u64)
            .or_else(|| {
                parsed
                    .get("data")
                    .and_then(|data| data.get("exitCode"))
                    .and_then(Value::as_u64)
            })
            .unwrap_or(0) as u32,
        stdout: parsed
            .get("stdout")
            .and_then(Value::as_str)
            .or_else(|| {
                parsed
                    .get("data")
                    .and_then(|data| data.get("stdout"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_string(),
        stderr: parsed
            .get("stderr")
            .and_then(Value::as_str)
            .or_else(|| {
                parsed
                    .get("data")
                    .and_then(|data| data.get("stderr"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_string(),
    }))
}

pub async fn sandbox_prompt(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxPromptRequest>,
) -> Result<TangleEvmResult<SandboxPromptResponse>, String> {
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

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/agents/run",
        &token,
        Value::Object(payload),
        crate::runtime::SidecarRuntimeConfig::load().timeout,
    )
    .await?;

    let (success, response, error, trace_id) = crate::extract_agent_fields(&parsed);

    Ok(TangleEvmResult(SandboxPromptResponse {
        success,
        response,
        error,
        trace_id,
        duration_ms: parsed
            .get("durationMs")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        input_tokens: parsed
            .get("usage")
            .and_then(|usage| usage.get("inputTokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: parsed
            .get("usage")
            .and_then(|usage| usage.get("outputTokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    }))
}

pub async fn sandbox_task(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxTaskRequest>,
) -> Result<TangleEvmResult<SandboxTaskResponse>, String> {
    let token = require_sidecar_token(&request.sidecar_token)?;
    require_sidecar_auth(&request.sidecar_url, &token)?;

    let mut request = request;
    request.sidecar_token = token;

    let response = run_task_request(
        &request,
        crate::runtime::SidecarRuntimeConfig::load().timeout,
    )
    .await?;
    Ok(TangleEvmResult(response))
}
