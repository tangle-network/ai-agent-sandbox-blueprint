use serde_json::json;

use crate::JsonResponse;
use crate::WorkflowControlRequest;
use crate::WorkflowCreateRequest;
use crate::tangle_evm::extract::{CallId, Caller, TangleEvmArg, TangleEvmResult};
use crate::workflows::{
    WorkflowEntry, apply_workflow_execution, resolve_next_run, run_workflow, workflow_tick,
    workflows,
};

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

    workflows()?
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

pub async fn workflow_trigger(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<WorkflowControlRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let entry = {
        let store = workflows()?
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

    let execution =
        run_workflow(&entry, crate::runtime::SidecarRuntimeConfig::load().timeout).await?;

    {
        let mut store = workflows()?
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

pub async fn workflow_cancel(
    Caller(_caller): Caller,
    TangleEvmArg(request): TangleEvmArg<WorkflowControlRequest>,
) -> Result<TangleEvmResult<JsonResponse>, String> {
    let mut store = workflows()?
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

pub async fn workflow_tick_job() -> Result<TangleEvmResult<JsonResponse>, String> {
    let response = workflow_tick(crate::runtime::SidecarRuntimeConfig::load().timeout).await?;
    Ok(TangleEvmResult(JsonResponse {
        json: response.to_string(),
    }))
}
