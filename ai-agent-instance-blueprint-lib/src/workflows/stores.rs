use super::*;

static WORKFLOWS: once_cell::sync::OnceCell<PersistentStore<WorkflowEntry>> =
    once_cell::sync::OnceCell::new();
static WORKFLOW_RUNTIME: once_cell::sync::OnceCell<PersistentStore<WorkflowRuntimeMetadata>> =
    once_cell::sync::OnceCell::new();

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

pub(crate) fn summarize_last_run_at(
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
