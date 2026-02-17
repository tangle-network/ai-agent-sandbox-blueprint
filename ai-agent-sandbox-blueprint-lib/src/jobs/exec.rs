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
pub async fn run_exec_request(request: &SandboxExecRequest) -> Result<SandboxExecResponse, String> {
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

    if let Some(record) = crate::runtime::get_sandbox_by_url_opt(&request.sidecar_url) {
        crate::runtime::touch_sandbox(&record.id);
    }

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
///
/// When `backend_profile` is provided, it is set as `backend.profile` so the
/// sidecar agent session uses it as persistent context. The profile can contain
/// `systemPrompt`, `resources.instructions`, `permission`, `memory`, etc.
pub fn build_agent_payload(
    message: &str,
    session_id: &str,
    model: &str,
    context_json: &str,
    timeout_ms: u64,
    extra_metadata: Option<Map<String, Value>>,
    backend_profile: Option<&Value>,
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

    let mut backend = Map::new();
    if !model.is_empty() {
        backend.insert("model".to_string(), Value::String(model.to_string()));
    }
    if let Some(profile) = backend_profile {
        if let Some(obj) = profile.as_object() {
            if !obj.is_empty() {
                backend.insert("profile".to_string(), profile.clone());
            }
        }
    }
    if !backend.is_empty() {
        payload.insert("backend".to_string(), Value::Object(backend));
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

/// Convert a plain system prompt string into a profile object with
/// `{"systemPrompt": "..."}`. Useful for backward compatibility.
pub fn system_prompt_to_profile(sp: &str) -> Value {
    json!({ "systemPrompt": sp })
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
    if let Some(record) = crate::runtime::get_sandbox_by_url_opt(sidecar_url) {
        crate::runtime::touch_sandbox(&record.id);
    }

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
pub async fn run_task_request(request: &SandboxTaskRequest) -> Result<SandboxTaskResponse, String> {
    run_task_request_with_profile(request, None).await
}

/// Run a task request with an optional system prompt that persists across the
/// sidecar agent session via `backend.profile.systemPrompt`.
///
/// This is a backward-compatible wrapper around `run_task_request_with_profile`.
pub async fn run_task_request_with_system_prompt(
    request: &SandboxTaskRequest,
    system_prompt: Option<&str>,
) -> Result<SandboxTaskResponse, String> {
    let profile = system_prompt
        .filter(|s| !s.is_empty())
        .map(system_prompt_to_profile);
    run_task_request_with_profile(request, profile.as_ref()).await
}

/// Run a task request with an optional full agent profile.
///
/// The profile is a JSON object set as `backend.profile` in the sidecar
/// `/agents/run` payload. It can contain `systemPrompt`, `resources.instructions`,
/// `permission`, `memory`, and other sidecar profile fields.
pub async fn run_task_request_with_profile(
    request: &SandboxTaskRequest,
    backend_profile: Option<&Value>,
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
        backend_profile,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_agent_payload_with_system_prompt() {
        let profile = system_prompt_to_profile("You are a trading expert.");
        let payload = build_agent_payload(
            "hello",
            "sess-1",
            "claude-haiku",
            "",
            0,
            None,
            Some(&profile),
        )
        .unwrap();

        let backend = payload.get("backend").unwrap().as_object().unwrap();
        assert_eq!(backend["model"], "claude-haiku");
        let p = backend["profile"].as_object().unwrap();
        assert_eq!(p["systemPrompt"], "You are a trading expert.");
    }

    #[test]
    fn test_build_agent_payload_without_profile() {
        let payload = build_agent_payload(
            "hello",
            "sess-1",
            "claude-haiku",
            "",
            0,
            None,
            None,
        )
        .unwrap();

        let backend = payload.get("backend").unwrap().as_object().unwrap();
        assert_eq!(backend["model"], "claude-haiku");
        assert!(backend.get("profile").is_none());
    }

    #[test]
    fn test_build_agent_payload_empty_profile_ignored() {
        let empty = json!({});
        let payload = build_agent_payload(
            "hello",
            "",
            "",
            "",
            0,
            None,
            Some(&empty),
        )
        .unwrap();

        // No backend at all since model is empty and profile is empty
        assert!(payload.get("backend").is_none());
    }

    #[test]
    fn test_build_agent_payload_full_profile() {
        let profile = json!({
            "name": "trading-dex",
            "resources": {
                "instructions": {
                    "content": "You have a persistent workspace.",
                    "name": "trading-instructions.md"
                }
            },
            "permission": {
                "bash": "allow",
                "edit": "allow"
            },
            "memory": { "enabled": true }
        });
        let payload = build_agent_payload(
            "trade now",
            "sess-2",
            "claude-sonnet",
            "",
            0,
            None,
            Some(&profile),
        )
        .unwrap();

        let backend = payload.get("backend").unwrap().as_object().unwrap();
        let p = backend["profile"].as_object().unwrap();
        assert!(p.get("systemPrompt").is_none(), "Full profile should not have systemPrompt");
        assert!(p.get("resources").is_some());
        assert_eq!(p["permission"]["bash"], "allow");
        assert_eq!(p["memory"]["enabled"], true);
    }

    #[test]
    fn test_system_prompt_to_profile() {
        let profile = system_prompt_to_profile("You are helpful.");
        let obj = profile.as_object().unwrap();
        assert_eq!(obj["systemPrompt"], "You are helpful.");
        assert_eq!(obj.len(), 1);
    }
}
