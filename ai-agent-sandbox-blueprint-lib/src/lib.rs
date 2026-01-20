//! AI Agent Sandbox Blueprint

use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::macros::debug_job;
use blueprint_sdk::tangle_evm::TangleEvmLayer;
use blueprint_sdk::tangle_evm::extract::{CallId, Caller, TangleEvmArg, TangleEvmResult};
use chrono::{TimeZone, Utc};
use cron::Schedule;
use once_cell::sync::OnceCell;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Job IDs for sandbox operations (write-only).
pub const JOB_SANDBOX_CREATE: u8 = 0;
pub const JOB_SANDBOX_STOP: u8 = 1;
pub const JOB_SANDBOX_RESUME: u8 = 2;
pub const JOB_SANDBOX_DELETE: u8 = 3;
pub const JOB_SANDBOX_SNAPSHOT: u8 = 4;

/// Job IDs for execution operations (write-only).
pub const JOB_EXEC: u8 = 10;
pub const JOB_PROMPT: u8 = 11;
pub const JOB_TASK: u8 = 12;

/// Job IDs for batch operations (write-only).
pub const JOB_BATCH_CREATE: u8 = 20;
pub const JOB_BATCH_TASK: u8 = 21;
pub const JOB_BATCH_EXEC: u8 = 22;
pub const JOB_BATCH_COLLECT: u8 = 23;

/// Job IDs for workflow operations (write-only).
pub const JOB_WORKFLOW_CREATE: u8 = 30;
pub const JOB_WORKFLOW_TRIGGER: u8 = 31;
pub const JOB_WORKFLOW_CANCEL: u8 = 32;
pub const JOB_WORKFLOW_TICK: u8 = 33;

/// Job IDs for SSH access operations (write-only).
pub const JOB_SSH_PROVISION: u8 = 40;
pub const JOB_SSH_REVOKE: u8 = 41;

const DEFAULT_SANDBOX_BASE_URL: &str = "https://agents.tangle.network";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BATCH_COUNT: u32 = 50;

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

    /// Sandbox snapshot request.
    struct SandboxSnapshotRequest {
        string sidecar_url;
        string destination;
        bool include_workspace;
        bool include_state;
        string sidecar_token;
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

    /// Task request for a sandbox sidecar.
    struct SandboxTaskRequest {
        string sidecar_url;
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Task response from sandbox sidecar.
    struct SandboxTaskResponse {
        bool success;
        string result;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
        string session_id;
    }

    /// Batch sandbox create request.
    struct BatchCreateRequest {
        uint32 count;
        SandboxCreateRequest template_request;
        address[] operators;
        string distribution;
    }

    /// Batch task request.
    struct BatchTaskRequest {
        string[] sidecar_urls;
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
        bool parallel;
        string aggregation;
        string sidecar_token;
    }

    /// Batch exec request.
    struct BatchExecRequest {
        string[] sidecar_urls;
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
        bool parallel;
        string sidecar_token;
    }

    /// Batch collect request.
    struct BatchCollectRequest {
        string batch_id;
    }

    /// Workflow create request.
    struct WorkflowCreateRequest {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
    }

    /// Workflow control request.
    struct WorkflowControlRequest {
        uint64 workflow_id;
    }

    /// SSH provision request.
    struct SshProvisionRequest {
        string sidecar_url;
        string username;
        string public_key;
        string sidecar_token;
    }

    /// SSH revoke request.
    struct SshRevokeRequest {
        string sidecar_url;
        string username;
        string public_key;
        string sidecar_token;
    }


}

#[derive(Clone, Debug)]
struct WorkflowEntry {
    id: u64,
    name: String,
    workflow_json: String,
    trigger_type: String,
    trigger_config: String,
    sandbox_config_json: String,
    active: bool,
    next_run_at: Option<u64>,
    last_run_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WorkflowTaskSpec {
    sidecar_url: String,
    prompt: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    max_turns: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    context_json: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    sidecar_token: Option<String>,
}

struct WorkflowExecution {
    response: Value,
    last_run_at: u64,
    next_run_at: Option<u64>,
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

#[derive(Clone, Debug)]
struct BatchRecord {
    id: String,
    kind: String,
    results: Value,
}

static WORKFLOWS: OnceCell<Mutex<HashMap<u64, WorkflowEntry>>> = OnceCell::new();
static BATCH_COUNTER: AtomicU64 = AtomicU64::new(1);
static BATCH_RESULTS: OnceCell<Mutex<HashMap<String, BatchRecord>>> = OnceCell::new();

fn workflows() -> Result<&'static Mutex<HashMap<u64, WorkflowEntry>>, String> {
    WORKFLOWS
        .get_or_try_init(|| Ok(Mutex::new(HashMap::new())))
        .map_err(|err: String| err)
}

fn batches() -> Result<&'static Mutex<HashMap<String, BatchRecord>>, String> {
    BATCH_RESULTS
        .get_or_try_init(|| Ok(Mutex::new(HashMap::new())))
        .map_err(|err: String| err)
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

fn resolve_sidecar_token(default_token: Option<&str>, override_token: &str) -> Option<String> {
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

fn extract_agent_fields(parsed: &Value) -> (bool, String, String, String) {
    let success = parsed
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let response = parsed
        .get("response")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|data| data.get("finalText"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let error = parsed
        .get("error")
        .and_then(|err| {
            err.get("message")
                .and_then(Value::as_str)
                .or_else(|| err.as_str())
        })
        .unwrap_or_default()
        .to_string();
    let trace_id = parsed
        .get("traceId")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|data| data.get("metadata"))
                .and_then(|meta| meta.get("traceId"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();

    (success, response, error, trace_id)
}

fn normalize_username(username: &str) -> Result<String, String> {
    let trimmed = username.trim();
    let name = if trimmed.is_empty() { "root" } else { trimmed };
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err("Invalid SSH username".to_string());
    }
    Ok(name.to_string())
}

fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn build_snapshot_command(
    destination: &str,
    include_workspace: bool,
    include_state: bool,
) -> Result<String, String> {
    let mut paths = Vec::new();
    if include_workspace {
        paths.push("/workspace");
    }
    if include_state {
        paths.push("/var/lib/sidecar");
    }
    if paths.is_empty() {
        return Err("Snapshot must include workspace or state".to_string());
    }

    let dest = shell_escape(destination);
    let targets = paths.join(" ");
    Ok(format!(
        "set -euo pipefail; tmp=$(mktemp /tmp/sandbox-snapshot.XXXXXX.tar.gz); \
tar -czf \"$tmp\" {targets}; \
curl -fsSL -X PUT --upload-file \"$tmp\" {dest}; \
rm -f \"$tmp\""
    ))
}

fn next_batch_id() -> String {
    let id = BATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("batch-{id}")
}

fn workflow_store() -> Result<&'static Mutex<HashMap<u64, WorkflowEntry>>, String> {
    workflows()
}

fn now_ts() -> u64 {
    Utc::now().timestamp().max(0) as u64
}

fn compute_next_run(cron_expr: &str, from_ts: u64) -> Result<u64, String> {
    let schedule =
        Schedule::from_str(cron_expr).map_err(|err| format!("Invalid cron expression: {err}"))?;
    let base = Utc
        .timestamp_opt(from_ts as i64, 0)
        .single()
        .ok_or_else(|| "Invalid timestamp".to_string())?;
    schedule
        .after(&base)
        .next()
        .map(|dt| dt.timestamp().max(0) as u64)
        .ok_or_else(|| "Cron expression has no future run times".to_string())
}

fn resolve_next_run(
    trigger_type: &str,
    trigger_config: &str,
    last_run_at: Option<u64>,
) -> Result<Option<u64>, String> {
    if trigger_type != "cron" {
        return Ok(None);
    }
    let start = last_run_at.unwrap_or_else(now_ts);
    Ok(Some(compute_next_run(trigger_config, start)?))
}

async fn run_task_request(
    config: &GatewayConfig,
    request: &SandboxTaskRequest,
) -> Result<SandboxTaskResponse, String> {
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);

    let mut payload = Map::new();
    payload.insert(
        "identifier".to_string(),
        Value::String("default-agent".to_string()),
    );
    payload.insert(
        "message".to_string(),
        Value::String(request.prompt.to_string()),
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

    let mut metadata = Map::new();
    if !request.context_json.trim().is_empty() {
        let context = parse_json_object(&request.context_json, "context_json")?;
        if let Some(Value::Object(context)) = context {
            metadata.extend(context);
        }
    }

    if request.max_turns > 0 {
        metadata.insert("maxTurns".to_string(), json!(request.max_turns));
        metadata.insert("maxSteps".to_string(), json!(request.max_turns));
    }

    if !metadata.is_empty() {
        payload.insert("metadata".to_string(), Value::Object(metadata));
    }

    if request.timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(request.timeout_ms));
    }

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/agents/run",
        token.as_deref(),
        Value::Object(payload),
        config.timeout,
    )
    .await?;

    let (success, result, error, trace_id) = extract_agent_fields(&parsed);
    let session_id = parsed
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or(request.session_id.as_str())
        .to_string();

    Ok(SandboxTaskResponse {
        success,
        result,
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
        session_id,
    })
}

async fn run_workflow(entry: &WorkflowEntry) -> Result<WorkflowExecution, String> {
    if entry.workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }

    let spec: WorkflowTaskSpec = serde_json::from_str(entry.workflow_json.as_str())
        .map_err(|err| format!("workflow_json must be valid task JSON: {err}"))?;

    let request = SandboxTaskRequest {
        sidecar_url: spec.sidecar_url,
        prompt: spec.prompt,
        session_id: spec.session_id.unwrap_or_default(),
        max_turns: spec.max_turns.unwrap_or(0),
        model: spec.model.unwrap_or_default(),
        context_json: spec.context_json.unwrap_or_default(),
        timeout_ms: spec.timeout_ms.unwrap_or(0),
        sidecar_token: spec.sidecar_token.unwrap_or_default(),
    };

    let config = GatewayConfig::load();
    let response = run_task_request(&config, &request).await?;

    let now = now_ts();
    let next_run_at = resolve_next_run(&entry.trigger_type, &entry.trigger_config, Some(now))?;

    Ok(WorkflowExecution {
        response: json!({
        "workflowId": entry.id,
        "name": entry.name,
        "status": if entry.active { "active" } else { "inactive" },
        "executedAt": now,
        "sandboxConfigJson": entry.sandbox_config_json,
        "task": {
            "success": response.success,
            "result": response.result,
            "error": response.error,
                "traceId": response.trace_id,
                "durationMs": response.duration_ms,
                "inputTokens": response.input_tokens,
                "outputTokens": response.output_tokens,
                "sessionId": response.session_id,
            }
        }),
        last_run_at: now,
        next_run_at,
    })
}

fn apply_workflow_execution(entry: &mut WorkflowEntry, execution: &WorkflowExecution) {
    entry.last_run_at = Some(execution.last_run_at);
    entry.next_run_at = execution.next_run_at;
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

async fn sidecar_post_json(
    sidecar_url: &str,
    path: &str,
    token: Option<&str>,
    payload: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let url = build_url(sidecar_url, path)?;
    let headers = auth_headers(token)?;
    let (_, body) = send_json(Method::POST, url, Some(payload), headers, timeout).await?;
    serde_json::from_str(&body).map_err(|err| format!("Invalid sidecar response JSON: {err}"))
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

async fn sandbox_create_raw(
    config: &GatewayConfig,
    request: &SandboxCreateRequest,
) -> Result<String, String> {
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

    Ok(body)
}

#[debug_job]
pub async fn sandbox_create(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let config = GatewayConfig::load();
    let body = sandbox_create_raw(&config, &request).await?;
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
pub async fn sandbox_snapshot(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxSnapshotRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.destination.trim().is_empty() {
        return Err("Snapshot destination is required".to_string());
    }

    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);
    let command = build_snapshot_command(
        &request.destination,
        request.include_workspace,
        request.include_state,
    )?;

    let payload = json!({
        "command": format!("sh -c {}", shell_escape(&command)),
    });

    let response = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        token.as_deref(),
        payload,
        config.timeout,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn sandbox_exec(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxExecRequest>,
) -> Result<TangleEvmResult<SandboxExecResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);

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
        let env_map = parse_json_object(&request.env_json, "env_json")?;
        if let Some(env_map) = env_map {
            payload.insert("env".to_string(), env_map);
        }
    }

    let parsed = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        token.as_deref(),
        Value::Object(payload),
        config.timeout,
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

#[debug_job]
pub async fn sandbox_prompt(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxPromptRequest>,
) -> Result<TangleEvmResult<SandboxPromptResponse>, String> {
    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);

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
        let context = parse_json_object(&request.context_json, "context_json")?;
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
        token.as_deref(),
        Value::Object(payload),
        config.timeout,
    )
    .await?;

    let (success, response, error, trace_id) = extract_agent_fields(&parsed);

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

#[debug_job]
pub async fn sandbox_task(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SandboxTaskRequest>,
) -> Result<TangleEvmResult<SandboxTaskResponse>, String> {
    let config = GatewayConfig::load();
    let response = run_task_request(&config, &request).await?;
    Ok(TangleEvmResult(response))
}

#[debug_job]
pub async fn batch_create(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.count == 0 {
        return Err("Batch count must be greater than zero".to_string());
    }
    if request.count > MAX_BATCH_COUNT {
        return Err(format!("Batch count exceeds maximum of {MAX_BATCH_COUNT}"));
    }

    let config = GatewayConfig::load();
    let mut results = Vec::with_capacity(request.count as usize);
    for _ in 0..request.count {
        let body = sandbox_create_raw(&config, &request.template_request).await?;
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::String(body));
        results.push(value);
    }

    let batch_id = next_batch_id();
    let record = BatchRecord {
        id: batch_id.clone(),
        kind: "create".to_string(),
        results: Value::Array(results.clone()),
    };
    batches()?
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?
        .insert(batch_id.clone(), record);

    let response = json!({
        "batchId": batch_id,
        "results": results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn batch_task(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchTaskRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch task requires at least one sidecar_url".to_string());
    }

    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);
    let mut results = Vec::with_capacity(request.sidecar_urls.len());

    for sidecar_url in request.sidecar_urls.iter() {
        let mut payload = Map::new();
        payload.insert(
            "identifier".to_string(),
            Value::String("default-agent".to_string()),
        );
        payload.insert(
            "message".to_string(),
            Value::String(request.prompt.to_string()),
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

        let mut metadata = Map::new();
        if !request.context_json.trim().is_empty() {
            let context = parse_json_object(&request.context_json, "context_json")?;
            if let Some(Value::Object(context)) = context {
                metadata.extend(context);
            }
        }

        if request.max_turns > 0 {
            metadata.insert("maxTurns".to_string(), json!(request.max_turns));
            metadata.insert("maxSteps".to_string(), json!(request.max_turns));
        }

        if !metadata.is_empty() {
            payload.insert("metadata".to_string(), Value::Object(metadata));
        }

        if request.timeout_ms > 0 {
            payload.insert("timeout".to_string(), json!(request.timeout_ms));
        }

        let parsed = sidecar_post_json(
            sidecar_url,
            "/agents/run",
            token.as_deref(),
            Value::Object(payload),
            config.timeout,
        )
        .await?;

        results.push(json!({
            "sidecarUrl": sidecar_url,
            "response": parsed,
        }));
    }

    let batch_id = next_batch_id();
    let record = BatchRecord {
        id: batch_id.clone(),
        kind: "task".to_string(),
        results: Value::Array(results.clone()),
    };
    batches()?
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?
        .insert(batch_id.clone(), record);

    let response = json!({
        "batchId": batch_id,
        "results": results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn batch_exec(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchExecRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.sidecar_urls.is_empty() {
        return Err("Batch exec requires at least one sidecar_url".to_string());
    }

    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);
    let mut results = Vec::with_capacity(request.sidecar_urls.len());

    for sidecar_url in request.sidecar_urls.iter() {
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
            let env_map = parse_json_object(&request.env_json, "env_json")?;
            if let Some(env_map) = env_map {
                payload.insert("env".to_string(), env_map);
            }
        }

        let parsed = sidecar_post_json(
            sidecar_url,
            "/exec",
            token.as_deref(),
            Value::Object(payload),
            config.timeout,
        )
        .await?;

        results.push(json!({
            "sidecarUrl": sidecar_url,
            "response": parsed,
        }));
    }

    let batch_id = next_batch_id();
    let record = BatchRecord {
        id: batch_id.clone(),
        kind: "exec".to_string(),
        results: Value::Array(results.clone()),
    };
    batches()?
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?
        .insert(batch_id.clone(), record);

    let response = json!({
        "batchId": batch_id,
        "results": results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn batch_collect(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<BatchCollectRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let store = batches()?;
    let mut store = store
        .lock()
        .map_err(|_| "Batch store poisoned".to_string())?;
    let record = store
        .remove(request.batch_id.as_str())
        .ok_or_else(|| "Batch not found".to_string())?;

    let response = json!({
        "batchId": record.id,
        "kind": record.kind,
        "results": record.results,
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn workflow_create(
    Caller(_caller): Caller,
    CallId(call_id): CallId,
    TangleEvmArg(request): TangleEvmArg<WorkflowCreateRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    if request.workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }

    let trigger_type = request.trigger_type.to_string();
    let trigger_config = request.trigger_config.to_string();
    let next_run_at = resolve_next_run(&trigger_type, &trigger_config, None)?;

    let entry = WorkflowEntry {
        id: call_id,
        name: request.name.to_string(),
        workflow_json: request.workflow_json.to_string(),
        trigger_type,
        trigger_config,
        sandbox_config_json: request.sandbox_config_json.to_string(),
        active: true,
        next_run_at,
        last_run_at: None,
    };

    workflow_store()?
        .lock()
        .map_err(|_| "Workflow store poisoned".to_string())?
        .insert(call_id, entry);

    let response = json!({
        "workflowId": call_id,
        "status": "active",
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn workflow_trigger(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<WorkflowControlRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let entry = {
        let store = workflow_store()?
            .lock()
            .map_err(|_| "Workflow store poisoned".to_string())?;
        let entry = store
            .get(&request.workflow_id)
            .ok_or_else(|| "Workflow not found".to_string())?;
        if !entry.active {
            return Err("Workflow is not active".to_string());
        }
        entry.clone()
    };

    let execution = run_workflow(&entry).await?;

    {
        let mut store = workflow_store()?
            .lock()
            .map_err(|_| "Workflow store poisoned".to_string())?;
        if let Some(entry) = store.get_mut(&request.workflow_id) {
            apply_workflow_execution(entry, &execution);
        }
    }

    Ok(TangleEvmResult(JsonResponse {
        json: execution.response.to_string(),
    }))
}

#[debug_job]
pub async fn workflow_cancel(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<WorkflowControlRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let mut store = workflow_store()?
        .lock()
        .map_err(|_| "Workflow store poisoned".to_string())?;
    let entry = store
        .get_mut(&request.workflow_id)
        .ok_or_else(|| "Workflow not found".to_string())?;
    entry.active = false;
    entry.next_run_at = None;

    let response = json!({
        "workflowId": entry.id,
        "status": "canceled",
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn workflow_tick() -> Result<TangleEvmResult<JsonResponse>, String> {
    let now = now_ts();
    let mut due = Vec::new();
    {
        let store = workflow_store()?
            .lock()
            .map_err(|_| "Workflow store poisoned".to_string())?;
        for entry in store.values() {
            if !entry.active {
                continue;
            }
            if entry.trigger_type != "cron" {
                continue;
            }
            if let Some(next_run_at) = entry.next_run_at {
                if next_run_at <= now {
                    due.push(entry.id);
                }
            }
        }
    }

    let mut executed = Vec::new();
    for workflow_id in due {
        let entry = {
            let store = workflow_store()?
                .lock()
                .map_err(|_| "Workflow store poisoned".to_string())?;
            let entry = match store.get(&workflow_id) {
                Some(entry) => entry,
                None => continue,
            };
            if !entry.active {
                continue;
            }
            entry.clone()
        };

        match run_workflow(&entry).await {
            Ok(execution) => {
                let mut store = workflow_store()?
                    .lock()
                    .map_err(|_| "Workflow store poisoned".to_string())?;
                if let Some(entry) = store.get_mut(&workflow_id) {
                    apply_workflow_execution(entry, &execution);
                }
                executed.push(execution.response);
            }
            Err(err) => executed.push(json!({
                "workflowId": workflow_id,
                "status": "error",
                "error": err,
            })),
        }
    }

    let response = json!({
        "executed": executed,
        "count": executed.len(),
    });

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn bootstrap_workflows_from_chain(
    client: &blueprint_sdk::contexts::tangle_evm::TangleEvmClient,
    service_id: u64,
) -> Result<(), String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| format!("Failed to get blueprint manager: {err}"))?;
    let Some(manager) = manager else {
        return Ok(());
    };

    let abi = blueprint_sdk::alloy::json_abi::JsonAbi::parse([WORKFLOW_REGISTRY_ABI])
        .map_err(|err| format!("Invalid workflow ABI: {err}"))?;
    let interface = blueprint_sdk::alloy::contract::Interface::new(abi);
    let contract = blueprint_sdk::alloy::contract::ContractInstance::new(
        manager,
        client.provider().clone(),
        interface,
    );

    let ids = contract
        .function(
            "getWorkflowIds",
            &[blueprint_sdk::alloy::dyn_abi::DynSolValue::Bool(true)],
        )
        .map_err(|err| format!("Failed to build workflow IDs call: {err}"))?
        .call()
        .await
        .map_err(|err| format!("Failed to read workflow IDs: {err}"))?;

    let ids = parse_workflow_ids(ids)?;
    let mut entries = HashMap::new();
    for workflow_id in ids {
        let output = contract
            .function(
                "getWorkflow",
                &[blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(
                    blueprint_sdk::alloy::primitives::U256::from_limbs([workflow_id, 0, 0, 0]),
                    64,
                )],
            )
            .map_err(|err| format!("Failed to build workflow {workflow_id} call: {err}"))?
            .call()
            .await
            .map_err(|err| format!("Failed to read workflow {workflow_id}: {err}"))?;
        let entry = parse_workflow_config(workflow_id, output)?;
        entries.insert(workflow_id, entry);
    }

    let mut store = workflow_store()?
        .lock()
        .map_err(|_| "Workflow store poisoned".to_string())?;
    *store = entries;
    Ok(())
}

fn parse_workflow_ids(
    values: Vec<blueprint_sdk::alloy::dyn_abi::DynSolValue>,
) -> Result<Vec<u64>, String> {
    let first = values
        .get(0)
        .ok_or_else(|| "Missing workflow IDs output".to_string())?;
    let blueprint_sdk::alloy::dyn_abi::DynSolValue::Array(ids) = first else {
        return Err("Unexpected workflow IDs output type".to_string());
    };
    let mut parsed = Vec::with_capacity(ids.len());
    for value in ids {
        let blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(id, _) = value else {
            return Err("Unexpected workflow ID type".to_string());
        };
        let id: u64 = (*id)
            .try_into()
            .map_err(|_| "Workflow ID overflow".to_string())?;
        parsed.push(id);
    }
    Ok(parsed)
}

fn parse_workflow_config(
    workflow_id: u64,
    values: Vec<blueprint_sdk::alloy::dyn_abi::DynSolValue>,
) -> Result<WorkflowEntry, String> {
    let first = values
        .get(0)
        .ok_or_else(|| "Missing workflow output".to_string())?;
    let blueprint_sdk::alloy::dyn_abi::DynSolValue::Tuple(fields) = first else {
        return Err("Unexpected workflow output type".to_string());
    };
    if fields.len() != 9 {
        return Err("Unexpected workflow tuple size".to_string());
    }

    let name = dyn_string(&fields[0])?;
    let workflow_json = dyn_string(&fields[1])?;
    let trigger_type = dyn_string(&fields[2])?;
    let trigger_config = dyn_string(&fields[3])?;
    let sandbox_config_json = dyn_string(&fields[4])?;
    let active = dyn_bool(&fields[5])?;
    let last_triggered_at = dyn_u64(&fields[8])?;
    let last_run_at = if last_triggered_at > 0 {
        Some(last_triggered_at)
    } else {
        None
    };
    let next_run_at = resolve_next_run(&trigger_type, &trigger_config, last_run_at)?;

    Ok(WorkflowEntry {
        id: workflow_id,
        name,
        workflow_json,
        trigger_type,
        trigger_config,
        sandbox_config_json,
        active,
        next_run_at,
        last_run_at,
    })
}

fn dyn_string(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<String, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::String(val) => Ok(val.to_string()),
        _ => Err("Unexpected string field type".to_string()),
    }
}

fn dyn_bool(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<bool, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Bool(val) => Ok(*val),
        _ => Err("Unexpected bool field type".to_string()),
    }
}

fn dyn_u64(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<u64, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(val, _) => (*val)
            .try_into()
            .map_err(|_| "Uint field overflow".to_string()),
        _ => Err("Unexpected uint field type".to_string()),
    }
}

const WORKFLOW_REGISTRY_ABI: &str = r#"[{"type":"function","name":"getWorkflowIds","inputs":[{"name":"activeOnly","type":"bool"}],"outputs":[{"name":"","type":"uint64[]"}],"stateMutability":"view"},{"type":"function","name":"getWorkflow","inputs":[{"name":"workflowId","type":"uint64"}],"outputs":[{"name":"","type":"tuple","components":[{"name":"name","type":"string"},{"name":"workflowJson","type":"string"},{"name":"triggerType","type":"string"},{"name":"triggerConfig","type":"string"},{"name":"sandboxConfigJson","type":"string"},{"name":"active","type":"bool"},{"name":"createdAt","type":"uint64"},{"name":"updatedAt","type":"uint64"},{"name":"lastTriggeredAt","type":"uint64"}]}],"stateMutability":"view"}]"#;

#[debug_job]
pub async fn ssh_provision(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SshProvisionRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let key = request.public_key.trim();
    if key.is_empty() {
        return Err("public_key is required".to_string());
    }

    let username = normalize_username(&request.username)?;
    let user_arg = shell_escape(&username);
    let key_arg = shell_escape(key);

    let command = format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6 || echo \"/root\"); \
mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
touch \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
if ! grep -qxF {key_arg} \"$home/.ssh/authorized_keys\"; then echo {key_arg} >> \"$home/.ssh/authorized_keys\"; fi"
    );

    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);
    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });

    let response = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        token.as_deref(),
        payload,
        config.timeout,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}

#[debug_job]
pub async fn ssh_revoke(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<SshRevokeRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let key = request.public_key.trim();
    if key.is_empty() {
        return Err("public_key is required".to_string());
    }

    let username = normalize_username(&request.username)?;
    let user_arg = shell_escape(&username);
    let key_arg = shell_escape(key);

    let command = format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6 || echo \"/root\"); \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    );

    let config = GatewayConfig::load();
    let token = resolve_sidecar_token(config.sidecar_token.as_deref(), &request.sidecar_token);
    let payload = json!({ "command": format!("sh -c {}", shell_escape(&command)) });

    let response = sidecar_post_json(
        &request.sidecar_url,
        "/exec",
        token.as_deref(),
        payload,
        config.timeout,
    )
    .await?;

    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
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
        .route(JOB_SANDBOX_SNAPSHOT, sandbox_snapshot.layer(TangleEvmLayer))
        .route(JOB_EXEC, sandbox_exec.layer(TangleEvmLayer))
        .route(JOB_PROMPT, sandbox_prompt.layer(TangleEvmLayer))
        .route(JOB_TASK, sandbox_task.layer(TangleEvmLayer))
        .route(JOB_BATCH_CREATE, batch_create.layer(TangleEvmLayer))
        .route(JOB_BATCH_TASK, batch_task.layer(TangleEvmLayer))
        .route(JOB_BATCH_EXEC, batch_exec.layer(TangleEvmLayer))
        .route(JOB_BATCH_COLLECT, batch_collect.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_CREATE, workflow_create.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_TRIGGER, workflow_trigger.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_CANCEL, workflow_cancel.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_TICK, workflow_tick)
        .route(JOB_SSH_PROVISION, ssh_provision.layer(TangleEvmLayer))
        .route(JOB_SSH_REVOKE, ssh_revoke.layer(TangleEvmLayer))
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
