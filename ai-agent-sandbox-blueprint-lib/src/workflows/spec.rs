use super::*;

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

pub(crate) fn resolve_workflow_sandbox(
    entry: &WorkflowEntry,
) -> Result<crate::SandboxRecord, String> {
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
