//! AI Agent Instance Blueprint
//!
//! Subscription-based blueprint: each service instance is a replicated AI agent
//! sandbox. Every operator in the service independently provisions and runs a
//! copy of the same sandbox configuration. Customers choose how many operators
//! (1 for simple use, N for redundancy/TEE verification). Each operator binary
//! manages its own single sandbox — no cross-operator coordination needed.
//! Exec/prompt/task jobs are instance-scoped: no sidecar URLs or tokens in
//! the request — the operator looks them up automatically.

#[cfg(feature = "billing")]
pub mod billing;
pub mod jobs;

// Re-export sandbox-runtime modules.
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
use once_cell::sync::OnceCell;
use serde_json::Value;

pub use blueprint_sdk::tangle;
pub use jobs::exec::{
    AgentResponse, build_agent_payload, build_exec_payload, call_agent, extract_exec_fields,
    instance_exec, instance_prompt, instance_task, parse_agent_response, run_instance_exec,
    run_instance_prompt, run_instance_task,
};
pub use jobs::provision::{
    deprovision_core, instance_deprovision, instance_provision, provision_core,
};
pub use jobs::snapshot::instance_snapshot;
pub use jobs::ssh::{instance_ssh_provision, instance_ssh_revoke, provision_key, revoke_key};

// ─────────────────────────────────────────────────────────────────────────────
// Job IDs
// ─────────────────────────────────────────────────────────────────────────────

pub const JOB_PROVISION: u8 = 0;
pub const JOB_EXEC: u8 = 1;
pub const JOB_PROMPT: u8 = 2;
pub const JOB_TASK: u8 = 3;
pub const JOB_SSH_PROVISION: u8 = 4;
pub const JOB_SSH_REVOKE: u8 = 5;
pub const JOB_SNAPSHOT: u8 = 6;
pub const JOB_DEPROVISION: u8 = 7;

// ─────────────────────────────────────────────────────────────────────────────
// ABI types
// ─────────────────────────────────────────────────────────────────────────────

sol! {
    struct JsonResponse {
        string json;
    }

    // ── Provision ──────────────────────────────────────────────────────────

    /// Instance provision request. Called once to create the sandbox.
    struct ProvisionRequest {
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
        /// TEE required flag.
        bool tee_required;
        /// TEE type: 0=None, 1=Tdx, 2=Nitro, 3=Sev.
        uint8 tee_type;
    }

    /// Provision output returned to customer.
    struct ProvisionOutput {
        string sandbox_id;
        string sidecar_url;
        uint32 ssh_port;
        string tee_attestation_json;
        /// TEE-bound public key JSON (empty for non-TEE sandboxes).
        /// Clients verify the attestation inside, then encrypt secrets to this key.
        string tee_public_key_json;
    }

    // ── Exec (no sidecar_url/token — instance-scoped) ─────────────────────

    struct InstanceExecRequest {
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
    }

    struct InstanceExecResponse {
        uint32 exit_code;
        string stdout;
        string stderr;
    }

    // ── Prompt (no sidecar_url/token — instance-scoped) ───────────────────

    struct InstancePromptRequest {
        string message;
        string session_id;
        string model;
        string context_json;
        uint64 timeout_ms;
    }

    struct InstancePromptResponse {
        bool success;
        string response;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
    }

    // ── Task (no sidecar_url/token — instance-scoped) ─────────────────────

    struct InstanceTaskRequest {
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
    }

    struct InstanceTaskResponse {
        bool success;
        string result;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
        string session_id;
    }

    // ── SSH (no sidecar_url/token — instance-scoped) ──────────────────────

    struct InstanceSshProvisionRequest {
        string username;
        string public_key;
    }

    struct InstanceSshRevokeRequest {
        string username;
        string public_key;
    }

    // ── Snapshot (no sidecar_url/token — instance-scoped) ─────────────────

    struct InstanceSnapshotRequest {
        string destination;
        bool include_workspace;
        bool include_state;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Instance state — singleton sandbox for this service instance
// ─────────────────────────────────────────────────────────────────────────────

static INSTANCE_STORE: OnceCell<store::PersistentStore<SandboxRecord>> = OnceCell::new();

const INSTANCE_KEY: &str = "instance";

/// Access the instance's persistent sandbox record store.
pub fn instance_store() -> error::Result<&'static store::PersistentStore<SandboxRecord>> {
    INSTANCE_STORE
        .get_or_try_init(|| {
            let path = store::state_dir().join("instance.json");
            store::PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

/// Get the provisioned sandbox record for this instance, if any.
pub fn get_instance_sandbox() -> error::Result<Option<SandboxRecord>> {
    instance_store()?.get(INSTANCE_KEY)
}

/// Get the provisioned sandbox or return an error if not yet provisioned.
pub fn require_instance_sandbox() -> Result<SandboxRecord, String> {
    get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Instance not provisioned — call JOB_PROVISION first".to_string())
}

/// Store the provisioned sandbox record.
pub fn set_instance_sandbox(record: SandboxRecord) -> error::Result<()> {
    instance_store()?.insert(INSTANCE_KEY.to_string(), record)
}

/// Remove the instance sandbox record.
pub fn clear_instance_sandbox() -> error::Result<()> {
    instance_store()?.remove(INSTANCE_KEY)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// ABI → runtime conversion
// ─────────────────────────────────────────────────────────────────────────────

impl From<&ProvisionRequest> for CreateSandboxParams {
    fn from(r: &ProvisionRequest) -> Self {
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

// ─────────────────────────────────────────────────────────────────────────────
// Agent response parsing (shared between prompt and task)
// ─────────────────────────────────────────────────────────────────────────────

/// Extract agent response fields from the sidecar `/agents/run` response.
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

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

pub fn router() -> Router {
    Router::new()
        .route(JOB_PROVISION, instance_provision.layer(TangleLayer))
        .route(JOB_EXEC, instance_exec.layer(TangleLayer))
        .route(JOB_PROMPT, instance_prompt.layer(TangleLayer))
        .route(JOB_TASK, instance_task.layer(TangleLayer))
        .route(JOB_SSH_PROVISION, instance_ssh_provision.layer(TangleLayer))
        .route(JOB_SSH_REVOKE, instance_ssh_revoke.layer(TangleLayer))
        .route(JOB_SNAPSHOT, instance_snapshot.layer(TangleLayer))
        .route(JOB_DEPROVISION, instance_deprovision.layer(TangleLayer))
}
