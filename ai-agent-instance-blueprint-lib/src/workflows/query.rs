use super::*;

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

    let effective_state = require_workflow_access(&entry, caller)?;

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
        match require_workflow_access(&entry, caller) {
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

    let effective_state = require_workflow_access(&entry, caller)?;
    workflow_detail_from_entry(&entry, effective_state)
}
