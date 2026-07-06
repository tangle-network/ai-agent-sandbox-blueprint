//! Instance workflow registry: on-chain sync, cron scheduling, gated
//! execution, and owner-scoped status queries for instance-targeted
//! workflows.

use chrono::{TimeZone, Utc};
use cron::Schedule;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Mutex;

use crate::InstanceTaskRequest;
use crate::jobs::exec::run_instance_task;
use crate::store::PersistentStore;
use crate::util::now_ts;

mod chain;
mod execution;
mod query;
mod run_guard;
mod schedule;
mod status;
mod stores;

#[cfg(test)]
pub(crate) use chain::WORKFLOW_REGISTRY_ABI;
pub(crate) use status::{
    merge_local_workflow_metadata, require_workflow_access, resolve_workflow_target_status,
    workflow_detail_from_entry, workflow_summary_from_entry,
};
pub(crate) use stores::{latest_execution_for_workflow, summarize_last_run_at};

pub use chain::bootstrap_workflows_from_chain;
pub use execution::{apply_workflow_execution, run_workflow, workflow_tick};
pub use query::{
    list_workflows_for_owner, workflow_detail_for_owner, workflow_runtime_status_for_owner,
};
pub use run_guard::{WorkflowRunGuard, acquire_workflow_run, is_workflow_running};
pub use schedule::resolve_next_run;
pub use stores::{
    store_failed_execution, store_latest_execution, workflow_key, workflow_runtime, workflows,
};

pub const WORKFLOW_TARGET_SANDBOX: u8 = 0;
pub const WORKFLOW_TARGET_INSTANCE: u8 = 1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkflowEntry {
    pub id: u64,
    pub name: String,
    pub workflow_json: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub sandbox_config_json: String,
    #[serde(default)]
    pub target_kind: u8,
    #[serde(default)]
    pub target_sandbox_id: String,
    #[serde(default)]
    pub target_service_id: u64,
    pub active: bool,
    pub next_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
    /// On-chain address of the caller who created this workflow.
    #[serde(default)]
    pub owner: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowLatestExecution {
    pub executed_at: u64,
    pub success: bool,
    pub result: String,
    pub error: String,
    pub trace_id: String,
    pub duration_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub session_id: String,
}

impl WorkflowLatestExecution {
    fn failed(executed_at: u64, error: String) -> Self {
        Self {
            executed_at,
            success: false,
            result: String::new(),
            error,
            trace_id: String::new(),
            duration_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            session_id: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeMetadata {
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeStatus {
    pub workflow_id: u64,
    pub target_status: WorkflowTargetStatus,
    pub runnable: bool,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub workflow_id: u64,
    pub name: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub target_kind: u8,
    pub target_sandbox_id: String,
    pub target_service_id: u64,
    pub active: bool,
    pub target_status: WorkflowTargetStatus,
    pub runnable: bool,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetail {
    pub workflow_id: u64,
    pub name: String,
    pub workflow_json: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub sandbox_config_json: String,
    pub target_kind: u8,
    pub target_sandbox_id: String,
    pub target_service_id: u64,
    pub active: bool,
    pub target_status: WorkflowTargetStatus,
    pub runnable: bool,
    pub running: bool,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub latest_execution: Option<WorkflowLatestExecution>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowTargetStatus {
    Available,
    Missing,
}

pub struct WorkflowExecution {
    pub response: Value,
    pub last_run_at: u64,
    pub next_run_at: Option<u64>,
    pub latest_execution: WorkflowLatestExecution,
}

#[derive(Debug)]
pub enum WorkflowStatusError {
    NotFound(String),
    Forbidden(String),
    Internal(String),
}

impl WorkflowStatusError {
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound(message) | Self::Forbidden(message) | Self::Internal(message) => {
                message.as_str()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowEffectiveState {
    pub(crate) target_status: WorkflowTargetStatus,
    pub(crate) runnable: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowTaskSpec {
    pub prompt: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context_json: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[cfg(test)]
mod tests;
