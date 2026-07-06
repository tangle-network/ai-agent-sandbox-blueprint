use chrono::{TimeZone, Utc};
use cron::Schedule;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Mutex;

use crate::SandboxTaskRequest;
use crate::auth::require_sidecar_token;
use crate::jobs::exec::run_task_request_with_profile;
use crate::runtime::require_sidecar_auth;
use crate::store::PersistentStore;
use crate::util::now_ts;

mod chain;
mod run;
mod schedule;
mod spec;
mod status;
mod store;

pub use chain::*;
pub use run::*;
pub use schedule::*;
pub use spec::*;
pub use status::*;
pub use store::*;

#[cfg(test)]
mod tests;

// Sidecar token in WorkflowTaskSpec is stored in the workflow JSON config
// (not on-chain ABI). It's validated against the stored sandbox record.

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
struct WorkflowEffectiveState {
    target_status: WorkflowTargetStatus,
    runnable: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowTaskSpec {
    #[serde(default)]
    pub sidecar_url: Option<String>,
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
    #[serde(default)]
    pub sidecar_token: Option<String>,
    /// Legacy: plain system prompt string. Superseded by `backend_profile_json`.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Full agent profile JSON (serialized). When set, takes priority over
    /// `system_prompt`. Contains `resources.instructions`, `permission`,
    /// `memory`, etc.
    #[serde(default)]
    pub backend_profile_json: Option<String>,
}
