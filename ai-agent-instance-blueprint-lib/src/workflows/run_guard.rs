use super::*;

/// Tracks workflow IDs that are currently executing to prevent concurrent runs.
static RUNNING_WORKFLOWS: once_cell::sync::Lazy<Mutex<HashSet<u64>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashSet::new()));

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
