use serde_json::json;

use crate::JsonResponse;
use crate::WorkflowControlRequest;
use crate::WorkflowCreateRequest;
use crate::tangle::extract::{CallId, Caller, TangleArg, TangleResult};
use crate::workflows::{
    WorkflowEntry, apply_workflow_execution, resolve_next_run, run_workflow, workflow_key,
    workflow_tick, workflows,
};

pub async fn workflow_create(
    Caller(caller): Caller,
    CallId(call_id): CallId,
    TangleArg(request): TangleArg<WorkflowCreateRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
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
        owner: super::caller_hex(&caller),
    };

    workflows()?
        .insert(workflow_key(call_id), entry)
        .map_err(|e| e.to_string())?;

    let response = json!({
        "workflowId": call_id,
        "status": "active",
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn workflow_trigger(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<WorkflowControlRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let key = workflow_key(request.workflow_id);
    let entry = workflows()?
        .get(&key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Workflow not found".to_string())?;

    if !entry.owner.is_empty() && !entry.owner.eq_ignore_ascii_case(&caller_hex) {
        return Err(format!("Caller {caller_hex} does not own workflow {}", request.workflow_id));
    }

    if !entry.active {
        return Err("Workflow is not active".to_string());
    }

    let execution = run_workflow(&entry).await?;

    let last_run_at = execution.last_run_at;
    let next_run_at = execution.next_run_at;
    let _ = workflows()?.update(&key, |e| {
        apply_workflow_execution(e, last_run_at, next_run_at);
    });

    Ok(TangleResult(JsonResponse {
        json: execution.response.to_string(),
    }))
}

pub async fn workflow_cancel(
    Caller(caller): Caller,
    TangleArg(request): TangleArg<WorkflowControlRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    let caller_hex = super::caller_hex(&caller);
    let key = workflow_key(request.workflow_id);

    let entry = workflows()?
        .get(&key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Workflow not found".to_string())?;

    if !entry.owner.is_empty() && !entry.owner.eq_ignore_ascii_case(&caller_hex) {
        return Err(format!("Caller {caller_hex} does not own workflow {}", request.workflow_id));
    }

    let found = workflows()?
        .update(&key, |entry| {
            entry.active = false;
            entry.next_run_at = None;
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err("Workflow not found".to_string());
    }

    let response = json!({
        "workflowId": request.workflow_id,
        "status": "canceled",
    });

    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}

pub async fn workflow_tick_job() -> Result<TangleResult<JsonResponse>, String> {
    let response = workflow_tick().await?;
    Ok(TangleResult(JsonResponse {
        json: response.to_string(),
    }))
}
