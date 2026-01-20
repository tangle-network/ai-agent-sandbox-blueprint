use chrono::{TimeZone, Utc};
use cron::Schedule;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use crate::SandboxTaskRequest;
use crate::auth::require_sidecar_token;
use crate::http::sidecar_post_json;
use crate::runtime::require_sidecar_auth;

#[derive(Clone, Debug)]
pub struct WorkflowEntry {
    pub id: u64,
    pub name: String,
    pub workflow_json: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub sandbox_config_json: String,
    pub active: bool,
    pub next_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
}

pub struct WorkflowExecution {
    pub response: Value,
    pub last_run_at: u64,
    pub next_run_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowTaskSpec {
    pub sidecar_url: String,
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
}

static WORKFLOWS: once_cell::sync::OnceCell<Mutex<HashMap<u64, WorkflowEntry>>> =
    once_cell::sync::OnceCell::new();

pub fn workflows() -> Result<&'static Mutex<HashMap<u64, WorkflowEntry>>, String> {
    WORKFLOWS
        .get_or_try_init(|| Ok(Mutex::new(HashMap::new())))
        .map_err(|err: String| err)
}

pub fn now_ts() -> u64 {
    Utc::now().timestamp().max(0) as u64
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

pub async fn run_workflow(
    entry: &WorkflowEntry,
    timeout: std::time::Duration,
) -> Result<WorkflowExecution, String> {
    if entry.workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }

    let spec: WorkflowTaskSpec = serde_json::from_str(entry.workflow_json.as_str())
        .map_err(|err| format!("workflow_json must be valid task JSON: {err}"))?;

    let token = require_sidecar_token(spec.sidecar_token.as_deref().unwrap_or(""))?;
    let _record = require_sidecar_auth(&spec.sidecar_url, &token)?;

    let request = SandboxTaskRequest {
        sidecar_url: spec.sidecar_url,
        prompt: spec.prompt,
        session_id: spec.session_id.unwrap_or_default(),
        max_turns: spec.max_turns.unwrap_or(0),
        model: spec.model.unwrap_or_default(),
        context_json: spec.context_json.unwrap_or_default(),
        timeout_ms: spec.timeout_ms.unwrap_or(0),
        sidecar_token: token,
    };

    let response = run_task_request(&request, timeout).await?;
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

pub fn apply_workflow_execution(entry: &mut WorkflowEntry, execution: &WorkflowExecution) {
    entry.last_run_at = Some(execution.last_run_at);
    entry.next_run_at = execution.next_run_at;
}

pub async fn run_task_request(
    request: &SandboxTaskRequest,
    timeout: std::time::Duration,
) -> Result<crate::SandboxTaskResponse, String> {
    let mut payload = serde_json::Map::new();
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

    let mut metadata = serde_json::Map::new();
    if !request.context_json.trim().is_empty() {
        let context = crate::util::parse_json_object(&request.context_json, "context_json")?;
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
        &request.sidecar_token,
        Value::Object(payload),
        timeout,
    )
    .await?;

    let (success, result, error, trace_id) = crate::extract_agent_fields(&parsed);
    let session_id = parsed
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or(request.session_id.as_str())
        .to_string();

    Ok(crate::SandboxTaskResponse {
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

pub async fn workflow_tick(timeout: std::time::Duration) -> Result<Value, String> {
    let now = now_ts();
    let mut due = Vec::new();
    {
        let store = workflows()?
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
            let store = workflows()?
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

        match run_workflow(&entry, timeout).await {
            Ok(execution) => {
                let mut store = workflows()?
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

    Ok(json!({
        "executed": executed,
        "count": executed.len(),
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

    let mut store = workflows()?
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
