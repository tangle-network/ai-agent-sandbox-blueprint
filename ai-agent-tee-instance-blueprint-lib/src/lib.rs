//! AI Agent TEE Instance Blueprint
//!
//! TEE-backed variant of the instance blueprint. Reuses all handlers from the
//! base instance blueprint except provision/deprovision, which route through
//! a `TeeBackend` (Phala CVM by default).
//!
//! Operators deploy this blueprint instead of the base instance blueprint when
//! they can provide TEE execution. The on-chain contract enforces higher pricing
//! (CVM costs) and attestation requirements.

pub mod jobs;

// Re-export from base instance blueprint — explicit to avoid leaking the base
// `router()` (callers should use `tee_router()`) and `jobs` module (shadowed
// by our own).
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
    JOB_DEPROVISION,
    JOB_EXEC,
    JOB_PROMPT,
    // Job IDs
    JOB_PROVISION,
    JOB_SNAPSHOT,
    JOB_SSH_PROVISION,
    JOB_SSH_REVOKE,
    JOB_TASK,
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
    // Reused job handlers
    instance_exec,
    instance_prompt,
    instance_snapshot,
    instance_ssh_provision,
    instance_ssh_revoke,
    // Instance state
    instance_store,
    instance_task,
    metrics,
    parse_agent_response,
    // Core functions (for composition)
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
};

use once_cell::sync::OnceCell;
use std::sync::Arc;

use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::tangle::TangleLayer;
use sandbox_runtime::tee::TeeBackend;

// ─────────────────────────────────────────────────────────────────────────────
// Global TEE backend
// ─────────────────────────────────────────────────────────────────────────────

static TEE_BACKEND: OnceCell<Arc<dyn TeeBackend>> = OnceCell::new();

/// Initialize the global TEE backend. Call once at startup.
pub fn init_tee_backend(backend: Arc<dyn TeeBackend>) {
    TEE_BACKEND
        .set(backend)
        .unwrap_or_else(|_| panic!("TEE backend already initialized"));
}

/// Get the global TEE backend. Panics if not yet initialized.
pub fn tee_backend() -> &'static Arc<dyn TeeBackend> {
    TEE_BACKEND
        .get()
        .expect("TEE backend not initialized — call init_tee_backend() first")
}

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

/// Build the TEE instance blueprint router.
///
/// Uses TEE-aware provision/deprovision handlers; all other handlers are
/// reused from the base instance blueprint.
pub fn tee_router() -> Router {
    use jobs::provision::{tee_deprovision, tee_provision};

    Router::new()
        .route(JOB_PROVISION, tee_provision.layer(TangleLayer))
        .route(JOB_EXEC, instance_exec.layer(TangleLayer))
        .route(JOB_PROMPT, instance_prompt.layer(TangleLayer))
        .route(JOB_TASK, instance_task.layer(TangleLayer))
        .route(JOB_SSH_PROVISION, instance_ssh_provision.layer(TangleLayer))
        .route(JOB_SSH_REVOKE, instance_ssh_revoke.layer(TangleLayer))
        .route(JOB_SNAPSHOT, instance_snapshot.layer(TangleLayer))
        .route(JOB_DEPROVISION, tee_deprovision.layer(TangleLayer))
}
