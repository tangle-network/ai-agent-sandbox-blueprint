use super::*;

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

pub(crate) fn latest_execution_for_workflow(
    workflow_id: u64,
) -> Result<Option<WorkflowLatestExecution>, String> {
    Ok(workflow_runtime()?
        .get(&workflow_key(workflow_id))
        .map_err(|e| e.to_string())?
        .and_then(|metadata| metadata.latest_execution))
}
