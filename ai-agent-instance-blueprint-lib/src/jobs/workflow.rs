use serde_json::json;

use crate::JsonResponse;
use crate::WorkflowControlRequest;
use crate::WorkflowCreateRequest;
use crate::tangle::extract::{CallId, Caller, ServiceId, TangleArg, TangleResult};
use crate::workflows::{
    WorkflowEntry, acquire_workflow_run, apply_workflow_execution, resolve_next_run, run_workflow,
    store_failed_execution, store_latest_execution, workflow_key, workflow_tick, workflows,
};

fn validate_instance_workflow_target(
    target_kind: u8,
    target_sandbox_id: &str,
    target_service_id: u64,
    service_id: u64,
) -> Result<u64, String> {
    if target_kind != crate::workflows::WORKFLOW_TARGET_INSTANCE {
        return Err("instance workflows must target an instance resource".to_string());
    }
    if !target_sandbox_id.trim().is_empty() {
        return Err("instance workflows must not set target_sandbox_id".to_string());
    }
    if target_service_id != 0 && target_service_id != service_id {
        return Err(format!(
            "instance workflows must target current service {service_id}"
        ));
    }

    Ok(service_id)
}

pub async fn workflow_create(
    Caller(caller): Caller,
    ServiceId(service_id): ServiceId,
    CallId(call_id): CallId,
    TangleArg(request): TangleArg<WorkflowCreateRequest>,
) -> Result<TangleResult<JsonResponse>, String> {
    if request.workflow_json.trim().is_empty() {
        return Err("workflow_json is required".to_string());
    }
    let target_service_id = validate_instance_workflow_target(
        request.target_kind,
        request.target_sandbox_id.as_str(),
        request.target_service_id,
        service_id,
    )?;

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
        target_kind: request.target_kind,
        target_sandbox_id: request.target_sandbox_id.to_string(),
        target_service_id,
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
        return Err(format!(
            "Caller {caller_hex} does not own workflow {}",
            request.workflow_id
        ));
    }

    if !entry.active {
        return Err("Workflow is not active".to_string());
    }

    let _run_guard = acquire_workflow_run(request.workflow_id)?;
    let execution = match run_workflow(&entry).await {
        Ok(execution) => execution,
        Err(err) => {
            store_failed_execution(request.workflow_id, err.clone())?;
            return Err(err);
        }
    };

    let last_run_at = execution.last_run_at;
    let next_run_at = execution.next_run_at;
    store_latest_execution(request.workflow_id, execution.latest_execution.clone())?;
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
        return Err(format!(
            "Caller {caller_hex} does not own workflow {}",
            request.workflow_id
        ));
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

#[cfg(test)]
mod tests {
    use super::validate_instance_workflow_target;

    #[test]
    fn instance_workflow_accepts_zero_service_id_and_normalizes() {
        let resolved = validate_instance_workflow_target(1, "", 0, 42).unwrap();
        assert_eq!(resolved, 42);
    }

    #[test]
    fn instance_workflow_rejects_non_empty_sandbox_id() {
        let err = validate_instance_workflow_target(1, "sb-1", 0, 42).unwrap_err();
        assert!(err.contains("must not set target_sandbox_id"));
    }

    #[test]
    fn instance_workflow_rejects_non_instance_target_kind() {
        let err = validate_instance_workflow_target(0, "", 0, 42).unwrap_err();
        assert!(err.contains("target an instance resource"));
    }

    #[test]
    fn instance_workflow_rejects_mismatched_service_id() {
        let err = validate_instance_workflow_target(1, "", 7, 42).unwrap_err();
        assert!(err.contains("current service 42"));
    }

    #[test]
    fn instance_workflow_rejects_completely_invalid_target_kind() {
        let err = validate_instance_workflow_target(2, "", 0, 42).unwrap_err();
        assert!(err.contains("target an instance resource"));
        let err = validate_instance_workflow_target(255, "", 0, 42).unwrap_err();
        assert!(err.contains("target an instance resource"));
    }

    #[test]
    fn instance_workflow_accepts_matching_service_id() {
        let resolved = validate_instance_workflow_target(1, "", 42, 42).unwrap();
        assert_eq!(resolved, 42);
    }

    /// Whitespace-only sandbox_id passes Rust validation (trim().is_empty() == true),
    /// but would fail on-chain where Solidity checks bytes(...).length != 0.
    #[test]
    fn instance_workflow_accepts_whitespace_only_sandbox_id() {
        let resolved = validate_instance_workflow_target(1, "   ", 0, 42).unwrap();
        assert_eq!(resolved, 42);
    }
}
