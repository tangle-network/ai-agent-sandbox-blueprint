//! AI Agent Sandbox Blueprint

use blueprint_sdk::alloy::sol;
use blueprint_sdk::macros::debug_job;
use blueprint_sdk::tangle_evm::TangleEvmLayer;
use blueprint_sdk::tangle_evm::extract::{Caller, TangleEvmArg, TangleEvmResult};
use blueprint_sdk::Job;
use blueprint_sdk::Router;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use serde_json::{Map, Value, json};
use std::env;
use once_cell::sync::OnceCell;
use std::time::Duration;

/// Job IDs for sandbox operations (write-only).
pub const JOB_SANDBOX_CREATE: u8 = 0;
pub const JOB_SANDBOX_DELETE: u8 = 3;
pub const JOB_SANDBOX_STOP: u8 = 4;
pub const JOB_SANDBOX_RESUME: u8 = 5;
pub const JOB_SANDBOX_EXEC: u8 = 6;
pub const JOB_SANDBOX_PROMPT: u8 = 7;

const DEFAULT_SANDBOX_BASE_URL: &str = "https://agents.tangle.network";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

sol! {
    /// Generic JSON response payload.
    struct JsonResponse {
        string json;
    }

    /// Sandbox create request.
    struct SandboxCreateRequest {
        string name;
        string image;
        string stack;
        string agent_identifier;
        string env_json;
        string metadata_json;
        bool ssh_enabled;
        string ssh_public_key;
        bool web_terminal_enabled;
        uint64 max_lifetime_seconds;
        uint64 idle_timeout_seconds;
        uint64 cpu_cores;
        uint64 memory_mb;
        uint64 disk_gb;
        string auth_token;
    }

    /// Sandbox identifier request.
    struct SandboxIdRequest {
        string sandbox_id;
        string auth_token;
    }

    /// Exec request for a sandbox sidecar.
    struct SandboxExecRequest {
        string sidecar_url;
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Exec response from sandbox sidecar.
    struct SandboxExecResponse {
        uint32 exit_code;
        string stdout;
        string stderr;
    }

    /// Prompt request for a sandbox sidecar.
    struct SandboxPromptRequest {
        string sidecar_url;
        string message;
        string session_id;
        string model;
        string context_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Prompt response from sandbox sidecar.
    struct SandboxPromptResponse {
        bool success;
        string response;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
    }

    
}

struct GatewayConfig {
    sandbox_base_url: String,
    sandbox_api_key: Option<String>,
    sidecar_token: Option<String>,
    timeout: Duration,
}

impl GatewayConfig {
    fn load() -> Self {
        let sandbox_base_url = env::var("SANDBOX_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_SANDBOX_BASE_URL.to_string());
        let sandbox_api_key = env::var("SANDBOX_API_KEY").ok();
        let sidecar_token = env::var("SIDECAR_TOKEN").ok();
        let timeout = env::var("REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        GatewayConfig {
            sandbox_base_url,
            sandbox_api_key,
            sidecar_token,
            timeout: Duration::from_secs(timeout),
        }
    }
}

static HTTP_CLIENT: OnceCell<reqwest::Client> = OnceCell::new();

fn http_client(timeout: Duration) -> Result<&'static reqwest::Client, String> {
    HTTP_CLIENT
        .get_or_try_init(|| {
            reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|err| format!("Failed to build HTTP client: {err}"))
        })
        .map_err(|err| err.to_string())
}

fn resolve_token(
    default_token: Option<&str>,
    override_token: &str,
    required: bool,
) -> Result<Option<String>, String> {
    let token = if override_token.trim().is_empty() {
        default_token.map(|value| value.to_string())
    } else {
        Some(override_token.trim().to_string())
    };

    if required && token.is_none() {
        return Err("Missing auth token".to_string());
    }

    Ok(token)
}

fn resolve_sidecar_token(
    default_token: Option<&str>,
    override_token: &str,
) -> Option<String> {
    if override_token.trim().is_empty() {
        default_token.map(|value| value.to_string())
    } else {
        Some(override_token.trim().to_string())
    }
}

fn parse_json_object(value: &str, field_name: &str) -> Result<Option<Value>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|err| format!("{field_name} is not valid JSON: {err}"))?;

    if !parsed.is_object() {
        return Err(format!("{field_name} must be a JSON object"));
    }

    Ok(Some(parsed))
}

fn build_url(base: &str, path: &str) -> Result<Url, String> {
    let base_url = Url::parse(base).map_err(|err| format!("Invalid base URL: {err}"))?;
    base_url
        .join(path)
        .map_err(|err| format!("Invalid path '{path}': {err}"))
}

fn auth_headers(token: Option<&str>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    if let Some(token) = token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Invalid auth token".to_string())?;
        headers.insert(AUTHORIZATION, value);
    }

    Ok(headers)
}

async fn send_json(
    method: Method,
    url: Url,
    body: Option<Value>,
    headers: HeaderMap,
    timeout: Duration,
) -> Result<(StatusCode, String), String> {
    let client = http_client(timeout)?;
    let mut request = client.request(method, url).headers(headers);
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request
        .send()
        .await
        .map_err(|err| format!("HTTP request failed: {err}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format!("Failed to read response body: {err}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }

    Ok((status, text))
}

 
fn merge_metadata(
    mut metadata: Option<Value>,
    image: &str,
    stack: &str,
) -> Result<Option<Value>, String> {
    if image.is_empty() && stack.is_empty() {
        return Ok(metadata);
    }

    let mut object = match metadata.take() {
        Some(Value::Object(map)) => map,
        Some(_) => return Err("metadata_json must be a JSON object".to_string()),
        None => Map::new(),
    };

    if !image.is_empty() {
        object.insert("image".to_string(), Value::String(image.to_string()));
    }

    if !stack.is_empty() {
        object.insert("stack".to_string(), Value::String(stack.to_string()));
    }

    Ok(Some(Value::Object(object)))
}

#[debug_job]
pub async fn sandbox_create(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_token(config.sandbox_api_key.as_deref(), &request.auth_token, true)?;

    let mut payload = Map::new();
    if !request.name.is_empty() {
        payload.insert("name".to_string(), Value::String(request.name.to_string()));
    }

    let mut config_fields = Map::new();
    if !request.agent_identifier.is_empty() {
        config_fields.insert(
            "agentIdentifier".to_string(),
            Value::String(request.agent_identifier.to_string()),
        );
    }

    if request.ssh_enabled {
        config_fields.insert("sshEnabled".to_string(), Value::Bool(true));
    }

    if !request.ssh_public_key.is_empty() {
        config_fields.insert(
            "sshPublicKey".to_string(),
            Value::String(request.ssh_public_key.to_string()),
        );
    }

    if request.web_terminal_enabled {
        config_fields.insert("webTerminalEnabled".to_string(), Value::Bool(true));
    }

    if request.max_lifetime_seconds > 0 {
        config_fields.insert(
            "maxLifetimeSeconds".to_string(),
            json!(request.max_lifetime_seconds),
        );
    }

    if request.idle_timeout_seconds > 0 {
        config_fields.insert(
            "idleTimeoutSeconds".to_string(),
            json!(request.idle_timeout_seconds),
        );
    }

    if !request.env_json.trim().is_empty() {
        let env_map = parse_json_object(&request.env_json, "env_json")?;
        if let Some(env_map) = env_map {
            config_fields.insert("env".to_string(), env_map);
        }
    }

    if request.cpu_cores > 0 || request.memory_mb > 0 || request.disk_gb > 0 {
        let mut resources = Map::new();
        if request.cpu_cores > 0 {
            resources.insert("cpuCores".to_string(), json!(request.cpu_cores));
        }
        if request.memory_mb > 0 {
            resources.insert("memoryMB".to_string(), json!(request.memory_mb));
        }
        if request.disk_gb > 0 {
            resources.insert("diskGB".to_string(), json!(request.disk_gb));
        }
        config_fields.insert("resources".to_string(), Value::Object(resources));
    }

    if !config_fields.is_empty() {
        payload.insert("config".to_string(), Value::Object(config_fields));
    }

    let metadata = parse_json_object(&request.metadata_json, "metadata_json")?;
    let metadata = merge_metadata(metadata, &request.image, &request.stack)?;
    if let Some(metadata) = metadata {
        payload.insert("metadata".to_string(), metadata);
    }

    let url = build_url(&config.sandbox_base_url, "/sandboxes")?;
    let headers = auth_headers(token.as_deref())?;

    let (_, body) = send_json(
        Method::POST,
        url,
        Some(Value::Object(payload)),
        headers,
        config.timeout,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse { json: body }))
}

#[debug_job]
pub async fn sandbox_delete(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_token(config.sandbox_api_key.as_deref(), &request.auth_token, true)?;

    let path = format!("/sandboxes/{}", request.sandbox_id);
    let url = build_url(&config.sandbox_base_url, &path)?;
    let headers = auth_headers(token.as_deref())?;

    let (_, body) = send_json(Method::DELETE, url, None, headers, config.timeout).await?;
    Ok(TangleEvmResult(JsonResponse { json: body }))
}

#[debug_job]
pub async fn sandbox_stop(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_token(config.sandbox_api_key.as_deref(), &request.auth_token, true)?;

    let path = format!("/sandboxes/{}/stop", request.sandbox_id);
    let url = build_url(&config.sandbox_base_url, &path)?;
    let headers = auth_headers(token.as_deref())?;

    let (_, body) = send_json(Method::POST, url, None, headers, config.timeout).await?;
    Ok(TangleEvmResult(JsonResponse { json: body }))
}

#[debug_job]
pub async fn sandbox_resume(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxIdRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_token(config.sandbox_api_key.as_deref(), &request.auth_token, true)?;

    let path = format!("/sandboxes/{}/resume", request.sandbox_id);
    let url = build_url(&config.sandbox_base_url, &path)?;
    let headers = auth_headers(token.as_deref())?;

    let (_, body) = send_json(Method::POST, url, None, headers, config.timeout).await?;
    Ok(TangleEvmResult(JsonResponse { json: body }))
}

#[debug_job]
pub async fn sandbox_exec(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxExecRequest>,
) -> Result<TangleEvmResult<SandboxExecResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);

    let url = build_url(&request.sidecar_url, "/exec")?;
    let headers = auth_headers(token.as_deref())?;

    let mut payload = Map::new();
    payload.insert("command".to_string(), Value::String(request.command.to_string()));
    if !request.cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(request.cwd.to_string()));
    }
    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }
    if !request.env_json.trim().is_empty() {
        let env_map = parse_json_object(&request.env_json, "env_json")?;
        if let Some(env_map) = env_map {
            payload.insert("env".to_string(), env_map);
        }
    }

    let (_, body) = send_json(
        Method::POST,
        url,
        Some(Value::Object(payload)),
        headers,
        config.timeout,
    )
    .await?;

    let parsed: Value = serde_json::from_str(&body)
        .map_err(|err| format!("Invalid exec response JSON: {err}"))?;

    Ok(TangleEvmResult(SandboxExecResponse {
        exit_code: parsed
            .get("exitCode")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        stdout: parsed
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: parsed
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }))
}

#[debug_job]
pub async fn sandbox_prompt(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxPromptRequest>,
) -> Result<TangleEvmResult<SandboxPromptResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);

    let url = build_url(&request.sidecar_url, "/agents/run")?;
    let headers = auth_headers(token.as_deref())?;

    let mut payload = Map::new();
    payload.insert("identifier".to_string(), Value::String("default-agent".to_string()));
    payload.insert("message".to_string(), Value::String(request.message.to_string()));

    if !request.session_id.is_empty() {
        payload.insert("sessionId".to_string(), Value::String(request.session_id.to_string()));
    }

    if !request.model.is_empty() {
        payload.insert(
            "backend".to_string(),
            json!({ "model": request.model }),
        );
    }

    if !request.context_json.trim().is_empty() {
        let context = parse_json_object(&request.context_json, "context_json")?;
        if let Some(context) = context {
            payload.insert("metadata".to_string(), context);
        }
    }

    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }

    let (_, body) = send_json(
        Method::POST,
        url,
        Some(Value::Object(payload)),
        headers,
        config.timeout,
    )
    .await?;

    let parsed: Value = serde_json::from_str(&body)
        .map_err(|err| format!("Invalid prompt response JSON: {err}"))?;

    Ok(TangleEvmResult(SandboxPromptResponse {
        success: parsed.get("success").and_then(Value::as_bool).unwrap_or(false),
        response: parsed
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        error: parsed
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        trace_id: parsed
            .get("traceId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
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

/// Router that maps job IDs to handlers.
#[must_use]
pub fn router() -> Router {
    Router::new()
        .route(JOB_SANDBOX_CREATE, sandbox_create.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_DELETE, sandbox_delete.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_STOP, sandbox_stop.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_RESUME, sandbox_resume.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_EXEC, sandbox_exec.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_PROMPT, sandbox_prompt.layer(TangleEvmLayer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_object_empty() {
        let result = parse_json_object("", "env_json").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_invalid() {
        let result = parse_json_object("[]", "env_json");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_token_prefers_override() {
        let token = resolve_token(Some("default"), "override", true).unwrap();
        assert_eq!(token.as_deref(), Some("override"));
    }
}
