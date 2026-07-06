use super::*;

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

fn workflow_effective_state_from_target_status(
    _entry: &WorkflowEntry,
    target_status: WorkflowTargetStatus,
) -> WorkflowEffectiveState {
    WorkflowEffectiveState {
        target_status,
        runnable: matches!(target_status, WorkflowTargetStatus::Available),
    }
}

fn owner_matches(entry: &WorkflowEntry, caller: &str) -> bool {
    !entry.owner.is_empty() && entry.owner.eq_ignore_ascii_case(caller)
}

fn workflow_summary_from_entry(
    entry: &WorkflowEntry,
    effective_state: WorkflowEffectiveState,
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
        target_status: effective_state.target_status,
        runnable: effective_state.runnable,
        running: effective_state.runnable && is_workflow_running(entry.id),
        last_run_at: summarize_last_run_at(entry, &latest_execution),
        next_run_at: effective_state
            .runnable
            .then_some(entry.next_run_at)
            .flatten(),
        latest_execution,
    })
}

fn workflow_detail_from_entry(
    entry: &WorkflowEntry,
    effective_state: WorkflowEffectiveState,
) -> Result<WorkflowDetail, WorkflowStatusError> {
    let summary = workflow_summary_from_entry(entry, effective_state)?;
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
        target_status: summary.target_status,
        runnable: summary.runnable,
        running: summary.running,
        last_run_at: summary.last_run_at,
        next_run_at: summary.next_run_at,
        latest_execution: summary.latest_execution,
    })
}

pub(crate) fn resolve_workflow_target_status(
    entry: &WorkflowEntry,
) -> Result<WorkflowTargetStatus, String> {
    if entry.target_kind == WORKFLOW_TARGET_SANDBOX && !entry.target_sandbox_id.trim().is_empty() {
        return match crate::runtime::get_sandbox_by_id(entry.target_sandbox_id.as_str()) {
            Ok(_) => Ok(WorkflowTargetStatus::Available),
            Err(crate::SandboxError::NotFound(_)) => Ok(WorkflowTargetStatus::Missing),
            Err(other) => Err(other.to_string()),
        };
    }

    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        "workflow_json must include sidecar_url when no sandbox target is provided".to_string()
    })?;

    match crate::runtime::get_sandbox_by_url(sidecar_url) {
        Ok(_) => Ok(WorkflowTargetStatus::Available),
        Err(crate::SandboxError::NotFound(_)) => Ok(WorkflowTargetStatus::Missing),
        Err(other) => Err(other.to_string()),
    }
}

fn resolve_workflow_effective_state_for_owner(
    entry: &WorkflowEntry,
    caller: &str,
) -> Result<WorkflowEffectiveState, WorkflowStatusError> {
    if entry.target_kind == WORKFLOW_TARGET_SANDBOX && !entry.target_sandbox_id.trim().is_empty() {
        return match crate::runtime::require_sandbox_owner(entry.target_sandbox_id.as_str(), caller)
        {
            Ok(_) => Ok(workflow_effective_state_from_target_status(
                entry,
                WorkflowTargetStatus::Available,
            )),
            Err(crate::SandboxError::NotFound(_)) if owner_matches(entry, caller) => Ok(
                workflow_effective_state_from_target_status(entry, WorkflowTargetStatus::Missing),
            ),
            Err(crate::SandboxError::NotFound(message)) => {
                Err(WorkflowStatusError::NotFound(message))
            }
            Err(crate::SandboxError::Auth(message)) => Err(WorkflowStatusError::Forbidden(message)),
            Err(other) => Err(WorkflowStatusError::Internal(other.to_string())),
        };
    }

    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())
        .map_err(WorkflowStatusError::Internal)?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        WorkflowStatusError::Internal(
            "workflow_json must include sidecar_url when no sandbox target is provided".to_string(),
        )
    })?;

    match crate::runtime::require_sandbox_owner_by_url(sidecar_url, caller) {
        Ok(_) => Ok(workflow_effective_state_from_target_status(
            entry,
            WorkflowTargetStatus::Available,
        )),
        Err(crate::SandboxError::NotFound(_)) if owner_matches(entry, caller) => Ok(
            workflow_effective_state_from_target_status(entry, WorkflowTargetStatus::Missing),
        ),
        Err(crate::SandboxError::NotFound(message)) => Err(WorkflowStatusError::NotFound(message)),
        Err(crate::SandboxError::Auth(message)) => Err(WorkflowStatusError::Forbidden(message)),
        Err(other) => Err(WorkflowStatusError::Internal(other.to_string())),
    }
}

fn resolve_workflow_owner(entry: &WorkflowEntry) -> Result<Option<String>, String> {
    if entry.target_kind == WORKFLOW_TARGET_SANDBOX && !entry.target_sandbox_id.trim().is_empty() {
        return match crate::runtime::get_sandbox_by_id(entry.target_sandbox_id.as_str()) {
            Ok(record) if !record.owner.is_empty() => Ok(Some(record.owner)),
            Ok(_) | Err(crate::SandboxError::NotFound(_)) => Ok(None),
            Err(other) => Err(other.to_string()),
        };
    }

    let spec = parse_workflow_task_spec(entry.workflow_json.as_str())?;
    let sidecar_url = spec.sidecar_url.as_deref().ok_or_else(|| {
        "workflow_json must include sidecar_url when no sandbox target is provided".to_string()
    })?;

    match crate::runtime::get_sandbox_by_url(sidecar_url) {
        Ok(record) if !record.owner.is_empty() => Ok(Some(record.owner)),
        Ok(_) | Err(crate::SandboxError::NotFound(_)) => Ok(None),
        Err(other) => Err(other.to_string()),
    }
}

pub(crate) fn merge_local_workflow_metadata(
    entry: &mut WorkflowEntry,
    existing: Option<&WorkflowEntry>,
) -> Result<(), String> {
    if let Some(existing) = existing.filter(|workflow| !workflow.owner.is_empty()) {
        entry.owner = existing.owner.clone();
        return Ok(());
    }

    if entry.owner.is_empty()
        && let Some(owner) = resolve_workflow_owner(entry)?
    {
        entry.owner = owner;
    }

    Ok(())
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

    let effective_state = resolve_workflow_effective_state_for_owner(&entry, caller)?;
    let latest_execution =
        latest_execution_for_workflow(workflow_id).map_err(WorkflowStatusError::Internal)?;

    Ok(WorkflowRuntimeStatus {
        workflow_id,
        target_status: effective_state.target_status,
        runnable: effective_state.runnable,
        running: effective_state.runnable && is_workflow_running(workflow_id),
        last_run_at: summarize_last_run_at(&entry, &latest_execution),
        next_run_at: effective_state
            .runnable
            .then_some(entry.next_run_at)
            .flatten(),
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
        match resolve_workflow_effective_state_for_owner(&entry, caller) {
            Ok(effective_state) => {
                visible.push(workflow_summary_from_entry(&entry, effective_state)?)
            }
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

    let effective_state = resolve_workflow_effective_state_for_owner(&entry, caller)?;
    workflow_detail_from_entry(&entry, effective_state)
}
