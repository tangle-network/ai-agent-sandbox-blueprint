//! AI Agent Sandbox Blueprint
//!
//! Event-driven multi-sandbox blueprint. For the shared container runtime
//! used by this and other blueprints, see `sandbox-runtime`.

pub mod jobs;
pub mod workflows;

// Re-export sandbox-runtime modules so existing consumers (job handlers,
// tests, binary crate) can keep using `crate::runtime::*`, `crate::auth::*`, etc.
pub use sandbox_runtime::{
    CreateSandboxParams, DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE,
    DEFAULT_SIDECAR_SSH_PORT, DEFAULT_TIMEOUT_SECS, SandboxError, SandboxRecord, SandboxState,
    TeeConfig, TeeType,
};
pub use sandbox_runtime::{auth, error, http, metrics, reaper, runtime, store, tee, util};

use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::tangle::TangleLayer;
use serde_json::Value;

pub use blueprint_sdk::tangle;
pub use jobs::batch::{batch_collect, batch_create, batch_exec, batch_task};
pub use jobs::exec::{
    build_exec_payload, extract_exec_fields, run_exec_request, run_prompt_request,
    run_task_request, run_task_request_with_profile, run_task_request_with_system_prompt,
    sandbox_exec, sandbox_prompt, sandbox_task, system_prompt_to_profile,
};
pub use jobs::sandbox::{
    sandbox_create, sandbox_delete, sandbox_resume, sandbox_snapshot, sandbox_stop,
};
pub use jobs::ssh::{provision_key, revoke_key, ssh_provision, ssh_revoke};
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

pub const MAX_BATCH_COUNT: u32 = 50;

sol! {
    /// Generic JSON response payload.
    struct JsonResponse {
        string json;
    }

    /// Sandbox create output with extractable sandboxId for on-chain routing.
    /// The contract decodes the first field to store sandboxId → operator mapping.
    struct SandboxCreateOutput {
        string sandboxId;
        string json;
    }

    /// Sandbox create request.
    ///
    /// Note: `sidecar_token` is generated server-side and never appears in
    /// on-chain calldata. Secrets (API keys, etc.) should be injected via the
    /// operator API's 2-phase secret provisioning endpoint after creation.
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
        /// TEE required flag. When true, sandbox is created inside a TEE.
        bool tee_required;
        /// TEE type preference: 0=None (operator chooses), 1=Tdx, 2=Nitro, 3=Sev.
        uint8 tee_type;
    }

    /// Sandbox identifier request.
    struct SandboxIdRequest {
        string sandbox_id;
    }

    /// Sandbox snapshot request.
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SandboxSnapshotRequest {
        string sidecar_url;
        string destination;
        bool include_workspace;
        bool include_state;
    }

    /// Exec request for a sandbox sidecar.
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SandboxExecRequest {
        string sidecar_url;
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
    }

    /// Exec response from sandbox sidecar.
    struct SandboxExecResponse {
        uint32 exit_code;
        string stdout;
        string stderr;
    }

    /// Prompt request for a sandbox sidecar.
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SandboxPromptRequest {
        string sidecar_url;
        string message;
        string session_id;
        string model;
        string context_json;
        uint64 timeout_ms;
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
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SandboxTaskRequest {
        string sidecar_url;
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
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
    ///
    /// Auth: the on-chain `Caller` must own all sandboxes at `sidecar_urls`.
    /// Sidecar tokens are looked up from stored records.
    struct BatchTaskRequest {
        string[] sidecar_urls;
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
    ///
    /// Auth: the on-chain `Caller` must own all sandboxes at `sidecar_urls`.
    /// Sidecar tokens are looked up from stored records.
    struct BatchExecRequest {
        string[] sidecar_urls;
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
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SshProvisionRequest {
        string sidecar_url;
        string username;
        string public_key;
    }

    /// SSH revoke request.
    ///
    /// Auth: the on-chain `Caller` must own the sandbox at `sidecar_url`.
    /// The sidecar token is looked up from the stored record.
    struct SshRevokeRequest {
        string sidecar_url;
        string username;
        string public_key;
    }
}

/// Convert an ABI `SandboxCreateRequest` into runtime-level `CreateSandboxParams`.
impl From<&SandboxCreateRequest> for CreateSandboxParams {
    fn from(r: &SandboxCreateRequest) -> Self {
        let tee_config = if r.tee_required {
            Some(TeeConfig {
                required: true,
                tee_type: match r.tee_type {
                    1 => TeeType::Tdx,
                    2 => TeeType::Nitro,
                    3 => TeeType::Sev,
                    _ => TeeType::None,
                },
            })
        } else {
            None
        };

        Self {
            name: r.name.to_string(),
            image: r.image.to_string(),
            stack: r.stack.to_string(),
            agent_identifier: r.agent_identifier.to_string(),
            env_json: r.env_json.to_string(),
            metadata_json: r.metadata_json.to_string(),
            ssh_enabled: r.ssh_enabled,
            ssh_public_key: r.ssh_public_key.to_string(),
            web_terminal_enabled: r.web_terminal_enabled,
            max_lifetime_seconds: r.max_lifetime_seconds,
            idle_timeout_seconds: r.idle_timeout_seconds,
            cpu_cores: r.cpu_cores,
            memory_mb: r.memory_mb,
            disk_gb: r.disk_gb,
            owner: String::new(), // Set by the job handler from Caller extractor
            tee_config,
            user_env_json: String::new(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchRecord {
    pub id: String,
    pub kind: String,
    pub results: Value,
    pub created_at: u64,
}

static BATCH_RESULTS: once_cell::sync::OnceCell<store::PersistentStore<BatchRecord>> =
    once_cell::sync::OnceCell::new();

pub fn batches() -> error::Result<&'static store::PersistentStore<BatchRecord>> {
    BATCH_RESULTS
        .get_or_try_init(|| {
            let path = store::state_dir().join("batches.json");
            store::PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

pub fn next_batch_id() -> String {
    format!("batch-{}", uuid::Uuid::new_v4())
}

// ─────────────────────────────────────────────────────────────────────────────
// Optional TEE backend (configured at startup when TEE_BACKEND is set)
// ─────────────────────────────────────────────────────────────────────────────

static TEE_BACKEND: once_cell::sync::OnceCell<
    std::sync::Arc<dyn sandbox_runtime::tee::TeeBackend>,
> = once_cell::sync::OnceCell::new();

/// Initialize the optional TEE backend. Call once at startup if `TEE_BACKEND` is set.
pub fn init_tee_backend(backend: std::sync::Arc<dyn sandbox_runtime::tee::TeeBackend>) {
    if TEE_BACKEND.set(backend).is_err() {
        tracing::warn!("TEE backend already initialized, ignoring duplicate init");
    }
}

/// Get the TEE backend, if configured. Returns `None` for non-TEE operators.
pub fn tee_backend() -> Option<&'static std::sync::Arc<dyn sandbox_runtime::tee::TeeBackend>> {
    TEE_BACKEND.get()
}

/// Extract agent response fields from the sidecar `/agents/run` response.
///
/// Response shape: `{ success, response, error, traceId, durationMs, usage, sessionId }`
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
                .and_then(|d| d.get("finalText"))
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
        .unwrap_or_default()
        .to_string();

    (success, response, error, trace_id)
}

/// Router that maps job IDs to handlers.
pub fn router() -> Router {
    Router::new()
        .route(JOB_SANDBOX_CREATE, sandbox_create.layer(TangleLayer))
        .route(JOB_SANDBOX_DELETE, sandbox_delete.layer(TangleLayer))
        .route(JOB_SANDBOX_STOP, sandbox_stop.layer(TangleLayer))
        .route(JOB_SANDBOX_RESUME, sandbox_resume.layer(TangleLayer))
        .route(JOB_SANDBOX_SNAPSHOT, sandbox_snapshot.layer(TangleLayer))
        .route(JOB_EXEC, sandbox_exec.layer(TangleLayer))
        .route(JOB_PROMPT, sandbox_prompt.layer(TangleLayer))
        .route(JOB_TASK, sandbox_task.layer(TangleLayer))
        .route(JOB_BATCH_CREATE, batch_create.layer(TangleLayer))
        .route(JOB_BATCH_TASK, batch_task.layer(TangleLayer))
        .route(JOB_BATCH_EXEC, batch_exec.layer(TangleLayer))
        .route(JOB_BATCH_COLLECT, batch_collect.layer(TangleLayer))
        .route(JOB_WORKFLOW_CREATE, workflow_create.layer(TangleLayer))
        .route(JOB_WORKFLOW_TRIGGER, workflow_trigger.layer(TangleLayer))
        .route(JOB_WORKFLOW_CANCEL, workflow_cancel.layer(TangleLayer))
        .route(JOB_WORKFLOW_TICK, workflow_tick_job)
        .route(JOB_SSH_PROVISION, ssh_provision.layer(TangleLayer))
        .route(JOB_SSH_REVOKE, ssh_revoke.layer(TangleLayer))
}

#[cfg(test)]
mod tests {
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
