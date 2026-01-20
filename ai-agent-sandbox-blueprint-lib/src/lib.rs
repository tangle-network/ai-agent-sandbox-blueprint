//! AI Agent Sandbox Blueprint

pub mod auth;
pub mod http;
pub mod jobs;
pub mod runtime;
pub mod util;
pub mod workflows;

use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::tangle_evm::TangleEvmLayer;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub use blueprint_sdk::tangle_evm;
pub use jobs::batch::{batch_collect, batch_create, batch_exec, batch_task};
pub use jobs::exec::{sandbox_exec, sandbox_prompt, sandbox_task};
pub use jobs::sandbox::{
    sandbox_create, sandbox_delete, sandbox_resume, sandbox_snapshot, sandbox_stop,
};
pub use jobs::ssh::{ssh_provision, ssh_revoke};
pub use jobs::workflow::{workflow_cancel, workflow_create, workflow_tick_job, workflow_trigger};
pub use workflows::bootstrap_workflows_from_chain;

/// Job IDs for sandbox operations (write-only).
pub const JOB_SANDBOX_CREATE: u8 = 0;
pub const JOB_SANDBOX_STOP: u8 = 1;
pub const JOB_SANDBOX_RESUME: u8 = 2;
pub const JOB_SANDBOX_DELETE: u8 = 3;
pub const JOB_SANDBOX_SNAPSHOT: u8 = 4;

/// Job IDs for execution operations (write-only).
pub const JOB_EXEC: u8 = 10;
pub const JOB_PROMPT: u8 = 11;
pub const JOB_TASK: u8 = 12;

/// Job IDs for batch operations (write-only).
pub const JOB_BATCH_CREATE: u8 = 20;
pub const JOB_BATCH_TASK: u8 = 21;
pub const JOB_BATCH_EXEC: u8 = 22;
pub const JOB_BATCH_COLLECT: u8 = 23;

/// Job IDs for workflow operations (write-only).
pub const JOB_WORKFLOW_CREATE: u8 = 30;
pub const JOB_WORKFLOW_TRIGGER: u8 = 31;
pub const JOB_WORKFLOW_CANCEL: u8 = 32;
pub const JOB_WORKFLOW_TICK: u8 = 33;

/// Job IDs for SSH access operations (write-only).
pub const JOB_SSH_PROVISION: u8 = 40;
pub const JOB_SSH_REVOKE: u8 = 41;

pub const DEFAULT_SIDECAR_IMAGE: &str = "ghcr.io/tangle-network/sidecar:latest";
pub const DEFAULT_SIDECAR_HTTP_PORT: u16 = 8080;
pub const DEFAULT_SIDECAR_SSH_PORT: u16 = 22;
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const MAX_BATCH_COUNT: u32 = 50;

sol! {
    /// Generic JSON response payload.
    struct JsonResponse {
        string json;
    }

    /// Sandbox create request.
    struct SandboxCreateRequest {
        string name;
        string image;
        string stack;
        string agent_identifier;
        string env_json;
        string metadata_json;
        bool ssh_enabled;
        string ssh_public_key;
        bool web_terminal_enabled;
        uint64 max_lifetime_seconds;
        uint64 idle_timeout_seconds;
        uint64 cpu_cores;
        uint64 memory_mb;
        uint64 disk_gb;
        string sidecar_token;
    }

    /// Sandbox identifier request.
    struct SandboxIdRequest {
        string sandbox_id;
    }

    /// Sandbox snapshot request.
    struct SandboxSnapshotRequest {
        string sidecar_url;
        string destination;
        bool include_workspace;
        bool include_state;
        string sidecar_token;
    }

    /// Exec request for a sandbox sidecar.
    struct SandboxExecRequest {
        string sidecar_url;
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Exec response from sandbox sidecar.
    struct SandboxExecResponse {
        uint32 exit_code;
        string stdout;
        string stderr;
    }

    /// Prompt request for a sandbox sidecar.
    struct SandboxPromptRequest {
        string sidecar_url;
        string message;
        string session_id;
        string model;
        string context_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Prompt response from sandbox sidecar.
    struct SandboxPromptResponse {
        bool success;
        string response;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
    }

    /// Task request for a sandbox sidecar.
    struct SandboxTaskRequest {
        string sidecar_url;
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
        string sidecar_token;
    }

    /// Task response from sandbox sidecar.
    struct SandboxTaskResponse {
        bool success;
        string result;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
        string session_id;
    }

    /// Batch sandbox create request.
    struct BatchCreateRequest {
        uint32 count;
        SandboxCreateRequest template_request;
        address[] operators;
        string distribution;
    }

    /// Batch task request.
    struct BatchTaskRequest {
        string[] sidecar_urls;
        string[] sidecar_tokens;
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
        bool parallel;
        string aggregation;
    }

    /// Batch exec request.
    struct BatchExecRequest {
        string[] sidecar_urls;
        string[] sidecar_tokens;
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
        bool parallel;
    }

    /// Batch collect request.
    struct BatchCollectRequest {
        string batch_id;
    }

    /// Workflow create request.
    struct WorkflowCreateRequest {
        string name;
        string workflow_json;
        string trigger_type;
        string trigger_config;
        string sandbox_config_json;
    }

    /// Workflow control request.
    struct WorkflowControlRequest {
        uint64 workflow_id;
    }

    /// SSH provision request.
    struct SshProvisionRequest {
        string sidecar_url;
        string username;
        string public_key;
        string sidecar_token;
    }

    /// SSH revoke request.
    struct SshRevokeRequest {
        string sidecar_url;
        string username;
        string public_key;
        string sidecar_token;
    }
}

#[derive(Clone, Debug)]
pub struct BatchRecord {
    pub id: String,
    pub kind: String,
    pub results: Value,
}

static BATCH_COUNTER: AtomicU64 = AtomicU64::new(1);
static BATCH_RESULTS: once_cell::sync::OnceCell<Mutex<HashMap<String, BatchRecord>>> =
    once_cell::sync::OnceCell::new();

pub fn batches() -> Result<&'static Mutex<HashMap<String, BatchRecord>>, String> {
    BATCH_RESULTS
        .get_or_try_init(|| Ok(Mutex::new(HashMap::new())))
        .map_err(|err: String| err)
}

pub fn next_batch_id() -> String {
    let id = BATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("batch-{id}")
}

pub fn extract_agent_fields(parsed: &Value) -> (bool, String, String, String) {
    let success = parsed
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let response = parsed
        .get("response")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|data| data.get("finalText"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let error = parsed
        .get("error")
        .and_then(|err| {
            err.get("message")
                .and_then(Value::as_str)
                .or_else(|| err.as_str())
        })
        .unwrap_or_default()
        .to_string();
    let trace_id = parsed
        .get("traceId")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .get("data")
                .and_then(|data| data.get("metadata"))
                .and_then(|meta| meta.get("traceId"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();

    (success, response, error, trace_id)
}

/// Router that maps job IDs to handlers.
#[must_use]
pub fn router() -> Router {
    Router::new()
        .route(JOB_SANDBOX_CREATE, sandbox_create.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_DELETE, sandbox_delete.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_STOP, sandbox_stop.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_RESUME, sandbox_resume.layer(TangleEvmLayer))
        .route(JOB_SANDBOX_SNAPSHOT, sandbox_snapshot.layer(TangleEvmLayer))
        .route(JOB_EXEC, sandbox_exec.layer(TangleEvmLayer))
        .route(JOB_PROMPT, sandbox_prompt.layer(TangleEvmLayer))
        .route(JOB_TASK, sandbox_task.layer(TangleEvmLayer))
        .route(JOB_BATCH_CREATE, batch_create.layer(TangleEvmLayer))
        .route(JOB_BATCH_TASK, batch_task.layer(TangleEvmLayer))
        .route(JOB_BATCH_EXEC, batch_exec.layer(TangleEvmLayer))
        .route(JOB_BATCH_COLLECT, batch_collect.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_CREATE, workflow_create.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_TRIGGER, workflow_trigger.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_CANCEL, workflow_cancel.layer(TangleEvmLayer))
        .route(JOB_WORKFLOW_TICK, workflow_tick_job)
        .route(JOB_SSH_PROVISION, ssh_provision.layer(TangleEvmLayer))
        .route(JOB_SSH_REVOKE, ssh_revoke.layer(TangleEvmLayer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_object_empty() {
        let result = crate::util::parse_json_object("", "env_json").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_invalid() {
        let result = crate::util::parse_json_object("[]", "env_json");
        assert!(result.is_err());
    }
}
