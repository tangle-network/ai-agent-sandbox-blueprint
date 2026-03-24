use chrono::{TimeZone, Utc};
use cron::Schedule;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Mutex;

use crate::SandboxTaskRequest;
use crate::auth::require_sidecar_token;
use crate::jobs::exec::run_task_request_with_profile;
use crate::runtime::require_sidecar_auth;
use crate::store::PersistentStore;
use crate::util::now_ts;

// Sidecar token in WorkflowTaskSpec is stored in the workflow JSON config
// (not on-chain ABI). It's validated against the stored sandbox record.

pub const WORKFLOW_TARGET_SANDBOX: u8 = 0;
pub const WORKFLOW_TARGET_INSTANCE: u8 = 1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkflowEntry {
    pub id: u64,
    pub name: String,
    pub workflow_json: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub sandbox_config_json: String,
    #[serde(default)]
    pub target_kind: u8,
    #[serde(default)]
    pub target_sandbox_id: String,
    #[serde(default)]
    pub target_service_id: u64,
    pub active: bool,
    pub next_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
    /// On-chain address of the caller who created this workflow.
    #[serde(default)]
    pub owner: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowLatestExecution {
    pub executed_at: u64,
    pub success: bool,
    pub result: String,
    pub error: String,
    pub trace_id: String,
    pub duration_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub session_id: String,
}

impl WorkflowLatestExecution {
    fn failed(executed_at: u64, error: String) -> Self {
        Self {
            executed_at,
            success: false,
            result: String::new(),
            error,
            trace_id: String::new(),
            duration_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            session_id: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeMetadata {
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeStatus {
    pub workflow_id: u64,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub workflow_id: u64,
    pub name: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub target_kind: u8,
    pub target_sandbox_id: String,
    pub target_service_id: u64,
    pub active: bool,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetail {
    pub workflow_id: u64,
    pub name: String,
    pub workflow_json: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub sandbox_config_json: String,
    pub target_kind: u8,
    pub target_sandbox_id: String,
    pub target_service_id: u64,
    pub active: bool,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

pub struct WorkflowExecution {
    pub response: Value,
    pub last_run_at: u64,
    pub next_run_at: Option<u64>,
    pub latest_execution: WorkflowLatestExecution,
}

#[derive(Debug)]
pub enum WorkflowStatusError {
    NotFound(String),
    Forbidden(String),
    Internal(String),
}

impl WorkflowStatusError {
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound(message) | Self::Forbidden(message) | Self::Internal(message) => {
                message.as_str()
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkflowTaskSpec {
    #[serde(default)]
    pub sidecar_url: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context_json: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub sidecar_token: Option<String>,
    /// Legacy: plain system prompt string. Superseded by `backend_profile_json`.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Full agent profile JSON (serialized). When set, takes priority over
    /// `system_prompt`. Contains `resources.instructions`, `permission`,
    /// `memory`, etc.
    #[serde(default)]
    pub backend_profile_json: Option<String>,
}

static WORKFLOWS: once_cell::sync::OnceCell<PersistentStore<WorkflowEntry>> =
    once_cell::sync::OnceCell::new();
static WORKFLOW_RUNTIME: once_cell::sync::OnceCell<PersistentStore<WorkflowRuntimeMetadata>> =
    once_cell::sync::OnceCell::new();

/// Tracks workflow IDs that are currently executing to prevent concurrent runs.
static RUNNING_WORKFLOWS: once_cell::sync::Lazy<Mutex<HashSet<u64>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashSet::new()));

pub fn workflows() -> Result<&'static PersistentStore<WorkflowEntry>, String> {
    WORKFLOWS
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("workflows.json");
            PersistentStore::open(path).map_err(|e| e.to_string())
        })
        .map_err(|err: String| err)
}

pub fn workflow_key(id: u64) -> String {
    id.to_string()
}

pub fn workflow_runtime() -> Result<&'static PersistentStore<WorkflowRuntimeMetadata>, String> {
    WORKFLOW_RUNTIME
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("workflow-runtime.json");
            PersistentStore::open(path).map_err(|e| e.to_string())
        })
        .map_err(|err: String| err)
}

pub struct WorkflowRunGuard {
    workflow_id: u64,
}

impl Drop for WorkflowRunGuard {
    fn drop(&mut self) {
        RUNNING_WORKFLOWS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.workflow_id);
    }
}

pub fn acquire_workflow_run(workflow_id: u64) -> Result<WorkflowRunGuard, String> {
    let mut running = RUNNING_WORKFLOWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if running.contains(&workflow_id) {
        return Err(format!("Workflow {workflow_id} is already running"));
    }
    running.insert(workflow_id);
    Ok(WorkflowRunGuard { workflow_id })
}

pub fn is_workflow_running(workflow_id: u64) -> bool {
    RUNNING_WORKFLOWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&workflow_id)
}

pub fn store_latest_execution(
    workflow_id: u64,
    latest_execution: WorkflowLatestExecution,
) -> Result<(), String> {
    let key = workflow_key(workflow_id);
    let updated = workflow_runtime()?
        .update(&key, |metadata| {
            metadata.latest_execution = Some(latest_execution.clone());
        })
        .map_err(|e| e.to_string())?;

    if !updated {
        workflow_runtime()?
            .insert(
                key,
                WorkflowRuntimeMetadata {
                    latest_execution: Some(latest_execution),
                },
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn store_failed_execution(
    workflow_id: u64,
    error: String,
) -> Result<WorkflowLatestExecution, String> {
    let latest_execution = WorkflowLatestExecution::failed(now_ts(), error);
    store_latest_execution(workflow_id, latest_execution.clone())?;
    Ok(latest_execution)
}

fn latest_execution_for_workflow(
    workflow_id: u64,
) -> Result<Option<WorkflowLatestExecution>, String> {
    Ok(workflow_runtime()?
        .get(&workflow_key(workflow_id))
        .map_err(|e| e.to_string())?
        .and_then(|metadata| metadata.latest_execution))
}

fn summarize_last_run_at(
    entry: &WorkflowEntry,
    latest_execution: &Option<WorkflowLatestExecution>,
) -> Option<u64> {
    let latest_run_at = latest_execution
        .as_ref()
        .map(|execution| execution.executed_at);
    match (entry.last_run_at, latest_run_at) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn workflow_summary_from_entry(
    entry: &WorkflowEntry,
) -> Result<WorkflowSummary, WorkflowStatusError> {
    let latest_execution =
        latest_execution_for_workflow(entry.id).map_err(WorkflowStatusError::Internal)?;
    Ok(WorkflowSummary {
        workflow_id: entry.id,
        name: entry.name.clone(),
        trigger_type: entry.trigger_type.clone(),
        trigger_config: entry.trigger_config.clone(),
        target_kind: entry.target_kind,
        target_sandbox_id: entry.target_sandbox_id.clone(),
        target_service_id: entry.target_service_id,
        active: entry.active,
        running: is_workflow_running(entry.id),
        last_run_at: summarize_last_run_at(entry, &latest_execution),
        next_run_at: entry.next_run_at,
        latest_execution,
    })
}

fn workflow_detail_from_entry(
    entry: &WorkflowEntry,
) -> Result<WorkflowDetail, WorkflowStatusError> {
    let summary = workflow_summary_from_entry(entry)?;
    Ok(WorkflowDetail {
        workflow_id: summary.workflow_id,
        name: summary.name,
        workflow_json: entry.workflow_json.clone(),
        trigger_type: summary.trigger_type,
        trigger_config: summary.trigger_config,
        sandbox_config_json: entry.sandbox_config_json.clone(),
        target_kind: summary.target_kind,
        target_sandbox_id: summary.target_sandbox_id,
        target_service_id: summary.target_service_id,
        active: summary.active,
        running: summary.running,
        last_run_at: summary.last_run_at,
        next_run_at: summary.next_run_at,
        latest_execution: summary.latest_execution,
    })
}

pub fn resolve_next_run(
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

pub fn parse_workflow_task_spec(workflow_json: &str) -> Result<WorkflowTaskSpec, String> {
    if workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }

    serde_json::from_str(workflow_json)
        .map_err(|err| format!("workflow_json must be valid task JSON: {err}"))
}

pub fn validate_workflow_execution_ready(workflow_json: &str) -> Result<WorkflowTaskSpec, String> {
    let spec = parse_workflow_task_spec(workflow_json)?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        "workflow_json must include sidecar_url when no sandbox target is provided".to_string()
    })?;
    let record = crate::runtime::get_sandbox_by_url(sidecar_url).map_err(|err| err.to_string())?;
    let effective_env = record.effective_env_json();
    let has_credentials = crate::runtime::workflow_runtime_credentials_available(&effective_env)
        .map_err(|err| err.to_string())?;
    if !has_credentials {
        return Err(
            "Workflow execution requires valid AI credentials in the sandbox environment."
                .to_string(),
        );
    }

    Ok(spec)
}

pub fn validate_workflow_execution_ready_with_target(
    workflow_json: &str,
    target_sandbox_id: &str,
) -> Result<WorkflowTaskSpec, String> {
    // New workflow creation paths validate target_kind / target_sandbox_id
    // before calling this helper. The empty-target fallback remains here so
    // older stored workflows that only reference sidecar_url still run.
    if target_sandbox_id.trim().is_empty() {
        return validate_workflow_execution_ready(workflow_json);
    }

    let spec = parse_workflow_task_spec(workflow_json)?;
    let record =
        crate::runtime::get_sandbox_by_id(target_sandbox_id).map_err(|err| err.to_string())?;
    let effective_env = record.effective_env_json();
    let has_credentials = crate::runtime::workflow_runtime_credentials_available(&effective_env)
        .map_err(|err| err.to_string())?;
    if !has_credentials {
        return Err(
            "Workflow execution requires ANTHROPIC_API_KEY or ZAI_API_KEY in the operator environment, or valid AI credentials in the sandbox environment."
                .to_string(),
        );
    }

    Ok(spec)
}

fn resolve_workflow_sandbox(entry: &WorkflowEntry) -> Result<crate::SandboxRecord, String> {
    if entry.target_kind == WORKFLOW_TARGET_SANDBOX && !entry.target_sandbox_id.trim().is_empty() {
        return crate::runtime::get_sandbox_by_id(entry.target_sandbox_id.as_str())
            .map_err(|err| err.to_string());
    }

    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        "workflow_json must include sidecar_url when no sandbox target is provided".to_string()
    })?;
    crate::runtime::get_sandbox_by_url(sidecar_url).map_err(|err| err.to_string())
}

fn resolve_workflow_sandbox_for_owner(
    entry: &WorkflowEntry,
    caller: &str,
) -> Result<crate::SandboxRecord, WorkflowStatusError> {
    if entry.target_kind == WORKFLOW_TARGET_SANDBOX && !entry.target_sandbox_id.trim().is_empty() {
        return crate::runtime::require_sandbox_owner(entry.target_sandbox_id.as_str(), caller)
            .map_err(|err| match err {
                crate::SandboxError::NotFound(message) => WorkflowStatusError::NotFound(message),
                crate::SandboxError::Auth(message) => WorkflowStatusError::Forbidden(message),
                other => WorkflowStatusError::Internal(other.to_string()),
            });
    }

    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())
        .map_err(WorkflowStatusError::Internal)?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        WorkflowStatusError::Internal(
            "workflow_json must include sidecar_url when no sandbox target is provided".to_string(),
        )
    })?;

    crate::runtime::require_sandbox_owner_by_url(sidecar_url, caller).map_err(|err| match err {
        crate::SandboxError::NotFound(message) => WorkflowStatusError::NotFound(message),
        crate::SandboxError::Auth(message) => WorkflowStatusError::Forbidden(message),
        other => WorkflowStatusError::Internal(other.to_string()),
    })
}

pub fn workflow_runtime_status_for_owner(
    workflow_id: u64,
    caller: &str,
) -> Result<WorkflowRuntimeStatus, WorkflowStatusError> {
    let key = workflow_key(workflow_id);
    let entry = workflows()
        .map_err(WorkflowStatusError::Internal)?
        .get(&key)
        .map_err(|e| WorkflowStatusError::Internal(e.to_string()))?
        .ok_or_else(|| WorkflowStatusError::NotFound("Workflow not found".to_string()))?;

    let _record = resolve_workflow_sandbox_for_owner(&entry, caller)?;
    let latest_execution =
        latest_execution_for_workflow(workflow_id).map_err(WorkflowStatusError::Internal)?;

    Ok(WorkflowRuntimeStatus {
        workflow_id,
        running: is_workflow_running(workflow_id),
        last_run_at: summarize_last_run_at(&entry, &latest_execution),
        next_run_at: entry.next_run_at,
        latest_execution,
    })
}

pub fn list_workflows_for_owner(caller: &str) -> Result<Vec<WorkflowSummary>, WorkflowStatusError> {
    let mut visible = Vec::new();

    for entry in workflows()
        .map_err(WorkflowStatusError::Internal)?
        .values()
        .map_err(|e| WorkflowStatusError::Internal(e.to_string()))?
    {
        match resolve_workflow_sandbox_for_owner(&entry, caller) {
            Ok(_) => visible.push(workflow_summary_from_entry(&entry)?),
            Err(WorkflowStatusError::Forbidden(_)) | Err(WorkflowStatusError::NotFound(_)) => {}
            Err(WorkflowStatusError::Internal(err)) => {
                return Err(WorkflowStatusError::Internal(err));
            }
        }
    }

    visible.sort_by(|left, right| {
        let left_sort = left
            .last_run_at
            .or_else(|| {
                left.latest_execution
                    .as_ref()
                    .map(|execution| execution.executed_at)
            })
            .or(left.next_run_at)
            .unwrap_or(0);
        let right_sort = right
            .last_run_at
            .or_else(|| {
                right
                    .latest_execution
                    .as_ref()
                    .map(|execution| execution.executed_at)
            })
            .or(right.next_run_at)
            .unwrap_or(0);
        right_sort
            .cmp(&left_sort)
            .then_with(|| right.workflow_id.cmp(&left.workflow_id))
    });

    Ok(visible)
}

pub fn workflow_detail_for_owner(
    workflow_id: u64,
    caller: &str,
) -> Result<WorkflowDetail, WorkflowStatusError> {
    let key = workflow_key(workflow_id);
    let entry = workflows()
        .map_err(WorkflowStatusError::Internal)?
        .get(&key)
        .map_err(|e| WorkflowStatusError::Internal(e.to_string()))?
        .ok_or_else(|| WorkflowStatusError::NotFound("Workflow not found".to_string()))?;

    let _record = resolve_workflow_sandbox_for_owner(&entry, caller)?;
    workflow_detail_from_entry(&entry)
}

pub async fn run_workflow(entry: &WorkflowEntry) -> Result<WorkflowExecution, String> {
    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())?;
    let record = resolve_workflow_sandbox(entry)?;

    // Look up token from sandbox record. Falls back to spec.sidecar_token for
    // backward compat with workflows created before 2-phase provisioning.
    let token = record.token.clone();
    if token.is_empty() {
        // Legacy path: use token from workflow spec
        let token_fallback = require_sidecar_token(spec.sidecar_token.as_deref().unwrap_or(""))?;
        let _record = require_sidecar_auth(&record.sidecar_url, &token_fallback)?;
    }

    // Session-per-tick: each execution gets a unique session so messages don't
    // accumulate in a single session forever. The stored session_id acts as a
    // prefix (e.g. "trading-bot123") and we append a timestamp suffix.
    let session_id = match spec.session_id {
        Some(ref base) if !base.is_empty() => {
            format!("{}-{}", base, chrono::Utc::now().timestamp())
        }
        _ => String::new(),
    };

    let sidecar_url = record.sidecar_url.clone();
    let request = SandboxTaskRequest {
        sidecar_url: sidecar_url.clone(),
        prompt: spec.prompt,
        session_id,
        max_turns: spec.max_turns.unwrap_or(0),
        model: spec.model.unwrap_or_default(),
        context_json: spec.context_json.unwrap_or_default(),
        timeout_ms: spec.timeout_ms.unwrap_or(0),
    };

    // Resolve backend profile: prefer backend_profile_json, fall back to
    // legacy system_prompt wrapped as a profile.
    let backend_profile: Option<Value> = spec
        .backend_profile_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .or_else(|| {
            spec.system_prompt
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|sp| json!({ "systemPrompt": sp }))
        });

    let response =
        run_task_request_with_profile(&request, &token, backend_profile.as_ref()).await?;
    let now = now_ts();
    let next_run_at = resolve_next_run(&entry.trigger_type, &entry.trigger_config, Some(now))?;
    let latest_execution = WorkflowLatestExecution {
        executed_at: now,
        success: response.success,
        result: response.result.clone(),
        error: response.error.clone(),
        trace_id: response.trace_id.clone(),
        duration_ms: response.duration_ms,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        session_id: response.session_id.clone(),
    };

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
        latest_execution,
    })
}

pub fn apply_workflow_execution(
    entry: &mut WorkflowEntry,
    last_run_at: u64,
    next_run_at: Option<u64>,
) {
    entry.last_run_at = Some(last_run_at);
    entry.next_run_at = next_run_at;
}

pub async fn workflow_tick() -> Result<Value, String> {
    let now = now_ts();
    let all = workflows()?.values().map_err(|e| e.to_string())?;

    let due: Vec<u64> = all
        .iter()
        .filter(|e| e.active && e.trigger_type == "cron")
        .filter_map(|e| e.next_run_at.filter(|&t| t <= now).map(|_| e.id))
        .collect();

    let mut executed = Vec::new();
    for workflow_id in due {
        let _run_guard = match acquire_workflow_run(workflow_id) {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Workflow {workflow_id} already running, skipping");
                continue;
            }
        };

        let key = workflow_key(workflow_id);
        let entry = match workflows()?.get(&key).map_err(|e| e.to_string())? {
            Some(e) if e.active => e,
            _ => continue,
        };

        // Advance next_run_at BEFORE starting the run to prevent duplicate
        // executions when the cron fires faster than the workflow completes.
        let tentative_next =
            resolve_next_run(&entry.trigger_type, &entry.trigger_config, Some(now))
                .ok()
                .flatten();
        workflows()?
            .update(&key, |e| {
                e.next_run_at = tentative_next;
            })
            .map_err(|e| e.to_string())?;

        match run_workflow(&entry).await {
            Ok(execution) => {
                let last_run_at = execution.last_run_at;
                let next_run_at = execution.next_run_at;
                store_latest_execution(workflow_id, execution.latest_execution.clone())?;
                workflows()?
                    .update(&key, |e| {
                        e.last_run_at = Some(last_run_at);
                        e.next_run_at = next_run_at;
                    })
                    .map_err(|e| e.to_string())?;
                executed.push(execution.response);
            }
            Err(err) => {
                store_failed_execution(workflow_id, err.clone())?;
                executed.push(json!({
                    "workflowId": workflow_id,
                    "status": "error",
                    "error": err,
                }));
            }
        }
    }

    Ok(json!({
        "executed": executed,
        "count": executed.len(),
    }))
}

pub async fn bootstrap_workflows_from_chain(
    client: &blueprint_sdk::contexts::tangle::TangleClient,
    service_id: u64,
) -> Result<(), String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| format!("Failed to get blueprint manager: {err}"))?;
    let Some(manager) = manager else {
        return Ok(());
    };

    let abi: blueprint_sdk::alloy::json_abi::JsonAbi = serde_json::from_str(WORKFLOW_REGISTRY_ABI)
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
            &[blueprint_sdk::alloy::dyn_abi::DynSolValue::Bool(false)],
        )
        .map_err(|err| format!("Failed to build workflow IDs call: {err}"))?
        .call()
        .await
        .map_err(|err| format!("Failed to read workflow IDs: {err}"))?;

    let ids = parse_workflow_ids(ids)?;
    let mut entries: HashMap<String, WorkflowEntry> = HashMap::new();
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
        entries.insert(workflow_key(workflow_id), entry);
    }

    workflows()?.replace(entries).map_err(|e| e.to_string())?;
    Ok(())
}

fn parse_workflow_ids(
    values: Vec<blueprint_sdk::alloy::dyn_abi::DynSolValue>,
) -> Result<Vec<u64>, String> {
    let first = values
        .first()
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
        .first()
        .ok_or_else(|| "Missing workflow output".to_string())?;
    let blueprint_sdk::alloy::dyn_abi::DynSolValue::Tuple(fields) = first else {
        return Err("Unexpected workflow output type".to_string());
    };
    if fields.len() != 12 {
        return Err("Unexpected workflow tuple size".to_string());
    }

    let name = dyn_string(&fields[0])?;
    let workflow_json = dyn_string(&fields[1])?;
    let trigger_type = dyn_string(&fields[2])?;
    let trigger_config = dyn_string(&fields[3])?;
    let sandbox_config_json = dyn_string(&fields[4])?;
    let target_kind = dyn_u8(&fields[5])?;
    let target_sandbox_id = dyn_string(&fields[6])?;
    let target_service_id = dyn_u64(&fields[7])?;
    let active = dyn_bool(&fields[8])?;
    let last_triggered_at = dyn_u64(&fields[11])?;
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
        target_kind,
        target_sandbox_id,
        target_service_id,
        active,
        next_run_at,
        last_run_at,
        owner: String::new(), // On-chain workflows don't have a caller context
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

fn dyn_u8(value: &blueprint_sdk::alloy::dyn_abi::DynSolValue) -> Result<u8, String> {
    match value {
        blueprint_sdk::alloy::dyn_abi::DynSolValue::Uint(val, _) => (*val)
            .try_into()
            .map_err(|_| "Uint field overflow".to_string()),
        _ => Err("Unexpected uint field type".to_string()),
    }
}

const WORKFLOW_REGISTRY_ABI: &str = r#"[{"type":"function","name":"getWorkflowIds","inputs":[{"name":"activeOnly","type":"bool"}],"outputs":[{"name":"","type":"uint64[]"}],"stateMutability":"view"},{"type":"function","name":"getWorkflow","inputs":[{"name":"workflowId","type":"uint64"}],"outputs":[{"name":"","type":"tuple","components":[{"name":"name","type":"string"},{"name":"workflowJson","type":"string"},{"name":"triggerType","type":"string"},{"name":"triggerConfig","type":"string"},{"name":"sandboxConfigJson","type":"string"},{"name":"targetKind","type":"uint8"},{"name":"targetSandboxId","type":"string"},{"name":"targetServiceId","type":"uint64"},{"name":"active","type":"bool"},{"name":"createdAt","type":"uint64"},{"name":"updatedAt","type":"uint64"},{"name":"lastTriggeredAt","type":"uint64"}]}],"stateMutability":"view"}]"#;

#[cfg(test)]
mod tests {
    use super::*;
    use blueprint_sdk::alloy::dyn_abi::DynSolValue;
    use blueprint_sdk::alloy::primitives::U256;

    #[test]
    fn dyn_string_extracts() {
        let val = DynSolValue::String("hello".into());
        assert_eq!(dyn_string(&val).unwrap(), "hello");
    }

    #[test]
    fn dyn_string_rejects_non_string() {
        let val = DynSolValue::Bool(true);
        assert!(dyn_string(&val).is_err());
    }

    #[test]
    fn dyn_bool_extracts() {
        assert!(dyn_bool(&DynSolValue::Bool(true)).unwrap());
        assert!(!dyn_bool(&DynSolValue::Bool(false)).unwrap());
    }

    #[test]
    fn dyn_bool_rejects_non_bool() {
        let val = DynSolValue::String("yes".into());
        assert!(dyn_bool(&val).is_err());
    }

    #[test]
    fn dyn_u64_extracts() {
        let val = DynSolValue::Uint(U256::from(42u64), 64);
        assert_eq!(dyn_u64(&val).unwrap(), 42);
    }

    #[test]
    fn workflow_registry_abi_parses() {
        let _: blueprint_sdk::alloy::json_abi::JsonAbi =
            serde_json::from_str(WORKFLOW_REGISTRY_ABI)
                .expect("workflow registry ABI should parse");
    }

    #[test]
    fn dyn_u64_overflow() {
        let val = DynSolValue::Uint(U256::MAX, 256);
        assert!(dyn_u64(&val).is_err());
    }

    #[test]
    fn parse_workflow_ids_empty() {
        let input = vec![DynSolValue::Array(vec![])];
        assert_eq!(parse_workflow_ids(input).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn parse_workflow_ids_multiple() {
        let input = vec![DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(1u64), 64),
            DynSolValue::Uint(U256::from(2u64), 64),
            DynSolValue::Uint(U256::from(3u64), 64),
        ])];
        assert_eq!(parse_workflow_ids(input).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn workflow_run_guard_tracks_running_state() {
        let workflow_id = u64::MAX - 41;
        assert!(!is_workflow_running(workflow_id));

        let guard = acquire_workflow_run(workflow_id).unwrap();
        assert!(is_workflow_running(workflow_id));
        assert!(acquire_workflow_run(workflow_id).is_err());

        drop(guard);
        assert!(!is_workflow_running(workflow_id));
    }
}
