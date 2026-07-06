use super::*;

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

pub(crate) fn resolve_workflow_target_status(
    entry: &WorkflowEntry,
) -> Result<WorkflowTargetStatus, String> {
    if entry.target_kind != WORKFLOW_TARGET_INSTANCE {
        return Ok(WorkflowTargetStatus::Missing);
    }

    match crate::get_instance_sandbox().map_err(|e| e.to_string())? {
        Some(record) => match record.service_id {
            Some(service_id) if service_id == entry.target_service_id => {
                Ok(WorkflowTargetStatus::Available)
            }
            Some(_) => Ok(WorkflowTargetStatus::Missing),
            None => Err("Local instance sandbox is missing service binding".to_string()),
        },
        None => Ok(WorkflowTargetStatus::Missing),
    }
}

pub(crate) fn require_workflow_access(
    entry: &WorkflowEntry,
    caller: &str,
) -> Result<WorkflowEffectiveState, WorkflowStatusError> {
    if entry.target_kind != WORKFLOW_TARGET_INSTANCE {
        return Err(WorkflowStatusError::NotFound(
            "Workflow target is not available on this operator".to_string(),
        ));
    }

    let Some(record) =
        crate::get_instance_sandbox().map_err(|e| WorkflowStatusError::Internal(e.to_string()))?
    else {
        if owner_matches(entry, caller) {
            return Ok(workflow_effective_state_from_target_status(
                entry,
                WorkflowTargetStatus::Missing,
            ));
        }
        return Err(WorkflowStatusError::NotFound(
            "Instance not provisioned".to_string(),
        ));
    };

    if record.owner.is_empty() {
        return Err(WorkflowStatusError::Forbidden(
            "Instance has no owner configured".to_string(),
        ));
    }
    if !record.owner.eq_ignore_ascii_case(caller) {
        return Err(WorkflowStatusError::Forbidden(
            "Not authorized for this instance".to_string(),
        ));
    }

    match record.service_id {
        Some(service_id) if service_id == entry.target_service_id => Ok(
            workflow_effective_state_from_target_status(entry, WorkflowTargetStatus::Available),
        ),
        Some(_) => Ok(workflow_effective_state_from_target_status(
            entry,
            WorkflowTargetStatus::Missing,
        )),
        None => Err(WorkflowStatusError::Internal(
            "Local instance sandbox is missing service binding".to_string(),
        )),
    }
}

pub(crate) fn workflow_summary_from_entry(
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

pub(crate) fn workflow_detail_from_entry(
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

fn resolve_workflow_owner(entry: &WorkflowEntry) -> Result<Option<String>, String> {
    if entry.target_kind != WORKFLOW_TARGET_INSTANCE {
        return Ok(None);
    }

    let Some(record) = crate::get_instance_sandbox().map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    match record.service_id {
        Some(service_id) if service_id == entry.target_service_id && !record.owner.is_empty() => {
            Ok(Some(record.owner))
        }
        Some(_) | None => Ok(None),
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
