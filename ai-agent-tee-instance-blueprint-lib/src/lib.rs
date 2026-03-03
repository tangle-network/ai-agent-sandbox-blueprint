//! AI Agent TEE Instance Blueprint
//!
//! TEE-backed variant of the instance blueprint. Reuses the same workflow and
//! API/runtime handlers as the base instance blueprint, with a configured
//! `TeeBackend` (Phala CVM by default) for local lifecycle operations.
//!
//! Operators deploy this blueprint instead of the base instance blueprint when
//! they can provide TEE execution. The on-chain contract enforces higher pricing
//! (CVM costs) and attestation requirements.

#[cfg(feature = "billing")]
pub use ai_agent_instance_blueprint_lib::billing;

// Re-export from base instance blueprint — explicit to avoid leaking the base
// `router()` (callers should use `tee_router()`) and `jobs` module (shadowed
// by our own).
pub use ai_agent_instance_blueprint_lib::auto_provision;
pub use ai_agent_instance_blueprint_lib::{
    AgentResponse,
    // Types
    CreateSandboxParams,
    DEFAULT_SIDECAR_HTTP_PORT,
    DEFAULT_SIDECAR_IMAGE,
    DEFAULT_SIDECAR_SSH_PORT,
    DEFAULT_TIMEOUT_SECS,
    InstanceExecRequest,
    InstanceExecResponse,
    InstancePromptRequest,
    InstancePromptResponse,
    InstanceSnapshotRequest,
    InstanceSshProvisionRequest,
    InstanceSshRevokeRequest,
    InstanceTaskRequest,
    InstanceTaskResponse,
    // Job IDs
    JOB_WORKFLOW_CANCEL,
    JOB_WORKFLOW_CREATE,
    JOB_WORKFLOW_TICK,
    JOB_WORKFLOW_TRIGGER,
    // ABI types
    JsonResponse,
    ProvisionOutput,
    ProvisionRequest,
    SandboxError,
    SandboxRecord,
    SandboxState,
    TeeConfig,
    TeeType,
    // Modules (runtime, store, reaper, etc.)
    auth,
    // Core functions (for composition)
    bootstrap_workflows_from_chain,
    // Exec helpers
    build_agent_payload,
    build_exec_payload,
    call_agent,
    clear_instance_sandbox,
    deprovision_core,
    error,
    // Agent response parsing
    extract_agent_fields,
    extract_exec_fields,
    get_instance_sandbox,
    http,
    // Instance state
    instance_store,
    metrics,
    parse_agent_response,
    provision_core,
    // SSH helpers
    provision_key,
    reaper,
    require_instance_sandbox,
    revoke_key,
    run_instance_exec,
    run_instance_prompt,
    run_instance_task,
    runtime,
    set_instance_sandbox,
    store,
    tangle,
    tee,
    util,
    workflow_cancel,
    workflow_create,
    workflow_tick_job,
    workflow_trigger,
};

use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::tangle::TangleLayer;

// Re-export TEE backend singleton from sandbox-runtime.
pub use sandbox_runtime::tee::{init_tee_backend, tee_backend};

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

/// Build the TEE instance blueprint router.
///
/// Uses the shared workflow handlers.
/// Read-only ops (exec, prompt, task, snapshot, SSH) are served via the
/// operator HTTP API.
pub fn tee_router() -> Router {
    Router::new()
        .route(JOB_WORKFLOW_CREATE, workflow_create.layer(TangleLayer))
        .route(JOB_WORKFLOW_TRIGGER, workflow_trigger.layer(TangleLayer))
        .route(JOB_WORKFLOW_CANCEL, workflow_cancel.layer(TangleLayer))
        .route(JOB_WORKFLOW_TICK, workflow_tick_job)
}
