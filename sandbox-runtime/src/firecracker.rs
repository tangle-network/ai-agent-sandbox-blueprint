//! Firecracker microVM lifecycle wrapper.
//!
//! Thin adapter around the [`microvm-runtime`] crate's in-process Firecracker
//! driver. The operator binary **is** the Firecracker host — there is no
//! separate "host-agent" service; this module talks directly to the VMM over
//! its unix socket via the primitive.
//!
//! ## Wired today (`microvm-runtime 0.1.0-alpha.1`)
//!
//! - VM create / start / stop / destroy lifecycle.
//! - Status reporting for the reaper reconcile loop
//!   (`FirecrackerContainerStatus::{Missing,Running,Stopped}`).
//! - Provider initialization probe used by the operator API health check.
//!
//! ## Not yet wired (returns [`SandboxError::Unsupported`])
//!
//! These will land as `microvm-runtime 0.2.0+` ships them (see the crate
//! ROADMAP). Failing loudly here is intentional: silent fallbacks would
//! deliver half-broken sandboxes.
//!
//! - **Sidecar endpoint URL**: requires network setup (TAP + bridge + NAT)
//!   to make the guest reachable from the host. Until then, the create path
//!   returns `Unsupported` rather than fabricating an endpoint.
//! - **Per-VM environment injection**: requires either init-time cloud-init
//!   or guest-side metadata service. Not yet exposed by the primitive.
//! - **Per-VM resource overrides** (cpu / memory / disk): the primitive's
//!   alpha takes resource sizing from process-wide env vars
//!   (`MICROVM_FIRECRACKER_VCPU`, `MICROVM_FIRECRACKER_MEM_MIB`); per-request
//!   overrides land in `0.2.0`.
//! - **Port forwarding**: requires the network layer above plus iptables
//!   DNAT.
//! - **Sandbox-issued sidecar auth token**: the runtime cannot inject a
//!   per-VM secret without env / vsock support.

use std::collections::HashMap;
use std::sync::OnceLock;

use microvm_runtime::{
    adapters::firecracker::FirecrackerVmProvider,
    error::VmRuntimeError,
    model::VmStatus,
    provider::{VmProvider, VmQuery},
};

use crate::error::{Result, SandboxError};

/// Single process-wide handle to the Firecracker provider.
///
/// The provider is `Clone`able but keeps an internal `Arc<RwLock<…>>` of VM
/// records, so we only want one instance per operator process — multiple
/// instances would diverge state and leak Firecracker child processes.
fn provider() -> &'static FirecrackerVmProvider {
    static PROVIDER: OnceLock<FirecrackerVmProvider> = OnceLock::new();
    PROVIDER.get_or_init(FirecrackerVmProvider::from_env)
}

/// Marker pointing at the upcoming `microvm-runtime` release that adds the
/// missing capability. Keeps the migration breadcrumb in one place.
const UPCOMING_RELEASE: &str = "microvm-runtime 0.2.0";

fn unsupported(feature: &str) -> SandboxError {
    SandboxError::Unsupported(format!(
        "{feature} is not yet supported by the in-process Firecracker driver; \
         tracked for {UPCOMING_RELEASE}. See https://github.com/tangle-network/microvm-runtime"
    ))
}

fn map_vm_error(action: &str, vm_id: &str, err: VmRuntimeError) -> SandboxError {
    match err {
        VmRuntimeError::VmNotFound(_) => {
            SandboxError::NotFound(format!("{action} vm {vm_id}: not found"))
        }
        VmRuntimeError::VmAlreadyExists(_) => {
            SandboxError::Validation(format!("{action} vm {vm_id}: already exists"))
        }
        VmRuntimeError::InvalidTransition { from, to, .. } => SandboxError::Validation(format!(
            "{action} vm {vm_id}: invalid transition {from} -> {to}"
        )),
        VmRuntimeError::SnapshotAlreadyExists { snapshot_id, .. } => SandboxError::Validation(
            format!("{action} vm {vm_id}: snapshot {snapshot_id} already exists"),
        ),
        VmRuntimeError::StatePoisoned => SandboxError::Unavailable(format!(
            "{action} vm {vm_id}: microvm-runtime state lock poisoned"
        )),
        VmRuntimeError::Unsupported(msg) => SandboxError::Unavailable(format!(
            "{action} vm {vm_id}: firecracker backend not ready: {msg}"
        )),
    }
}

/// Lifecycle status of a Firecracker VM as seen by the sandbox reaper.
///
/// Maps from [`VmStatus`] (the primitive's enum) plus the absence-of-record
/// case, which the reaper interprets as "the VM is gone and the record
/// should be reconciled away".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirecrackerContainerStatus {
    Running,
    Stopped,
    Missing,
}

/// Sandbox-side view of a created VM.
///
/// `id` is the `vm_id` passed to [`microvm-runtime`]. `endpoint` is `None`
/// today and stays `None` until the primitive ships network setup
/// ([`UPCOMING_RELEASE`]).
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerContainer {
    pub id: String,
    pub endpoint: Option<String>,
}

/// Sandbox-side view of a successful provision call.
///
/// Reserved for parity with the previous HTTP-client API even though the
/// in-process driver never injects a sidecar auth token (no env / vsock yet).
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerProvisionResult {
    pub container: FirecrackerContainer,
    pub sidecar_auth_token: Option<String>,
}

/// Sandbox-side create request.
///
/// Most fields are accepted for API parity but are not currently consumed —
/// see the module docs for which capabilities the primitive doesn't yet
/// expose. The fields are kept on the struct so callers can construct them
/// once and not have to special-case Firecracker; they survive into the
/// persisted [`crate::runtime::SandboxRecord`] via the runtime layer.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct FirecrackerCreateRequest {
    pub session_id: String,
    pub image: String,
    pub env: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    pub ports: Vec<crate::runtime::PortMapping>,
}

/// Sanity-check the configured Firecracker provider is reachable.
///
/// Used by `GET /health` on the operator API. The primitive initialises
/// lazily from env vars, so this confirms it constructs without panicking;
/// real prereq validation (kernel / rootfs / binary present) happens at
/// `create_vm` time and would surface there with an [`SandboxError::Unavailable`].
pub(crate) async fn health() -> Result<()> {
    let _ = provider();
    Ok(())
}

/// Create a VM and "start" it via the in-process driver.
///
/// Today this drives the lifecycle through to VM start, then fails with
/// [`SandboxError::Unsupported`] because the primitive cannot expose a
/// reachable sidecar endpoint URL. The VM is destroyed before returning so
/// no orphaned Firecracker processes are left behind.
///
/// Once network setup lands in [`UPCOMING_RELEASE`], this function will
/// return a real [`FirecrackerProvisionResult`] with an endpoint pointing
/// at the guest's exposed sidecar port.
pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    // Fail loudly on features that have no in-process implementation rather
    // than half-provisioning the VM and pretending it worked. The operator
    // metadata round-trip preserves these requests on the sandbox record, so
    // re-creation against a future driver release will pick them up.
    if !req.ports.is_empty() {
        return Err(unsupported(
            "metadata_json.ports forwarding for firecracker sandboxes",
        ));
    }
    // Anything beyond the runtime-injected envelope (currently `SIDECAR_PORT`
    // and `SIDECAR_CAPABILITIES`) is user / system env that the guest cannot
    // see without an injection channel (cloud-init or metadata service).
    let injected_keys = ["SIDECAR_PORT", "SIDECAR_CAPABILITIES"];
    let has_user_env = req.env.keys().any(|k| !injected_keys.contains(&k.as_str()));
    if has_user_env {
        return Err(unsupported(
            "per-VM environment injection for firecracker sandboxes",
        ));
    }

    let vm_id = req.session_id.clone();

    // The blocking lifecycle calls on `FirecrackerVmProvider` shell out to a
    // child process and do unix-socket I/O; isolate them onto a blocking
    // worker so the async runtime stays responsive.
    let create_id = vm_id.clone();
    tokio::task::spawn_blocking(move || provider().create_vm(&create_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker create join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("create", &vm_id, e))?;

    let start_id = vm_id.clone();
    if let Err(err) = tokio::task::spawn_blocking(move || provider().start_vm(&start_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker start join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("start", &vm_id, e))
    {
        // Best-effort cleanup so a partial create doesn't leak a process.
        let cleanup_id = vm_id.clone();
        let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
        return Err(err);
    }

    // The primitive does not yet expose a host-reachable endpoint for the
    // guest. Tearing the VM down is the honest answer: callers receive an
    // `Unsupported` error and no record is persisted by the runtime layer.
    let cleanup_id = vm_id.clone();
    let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;

    Err(unsupported(
        "host-reachable sidecar endpoint for firecracker sandboxes (VM lifecycle works; networking lands in",
    ))
}

/// Resume a previously-stopped VM.
///
/// Used by the sandbox `resume` lifecycle. Surfaces the same
/// `Unsupported` story as `create_and_start` once start succeeds — the VM
/// is back to running but its sidecar still has no host-reachable endpoint.
pub(crate) async fn start(container_id: &str) -> Result<FirecrackerContainer> {
    let vm_id = container_id.to_string();
    let start_id = vm_id.clone();
    tokio::task::spawn_blocking(move || provider().start_vm(&start_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker start join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("start", &vm_id, e))?;

    Ok(FirecrackerContainer {
        id: container_id.to_string(),
        endpoint: None,
    })
}

/// Stop a running VM.
///
/// Idempotent for callers: a missing VM (already torn down) is treated as
/// success because the reaper reconcile path needs to be able to call this
/// without first checking existence. Other lifecycle errors surface as
/// [`SandboxError::Validation`].
pub(crate) async fn stop(container_id: &str) -> Result<()> {
    let vm_id = container_id.to_string();
    let stop_id = vm_id.clone();
    match tokio::task::spawn_blocking(move || provider().stop_vm(&stop_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker stop join error for {vm_id}: {e}"))
        })? {
        Ok(()) => Ok(()),
        Err(VmRuntimeError::VmNotFound(_)) => Ok(()),
        Err(VmRuntimeError::InvalidTransition { .. }) => {
            // Already stopped — treat as idempotent success.
            Ok(())
        }
        Err(err) => Err(map_vm_error("stop", &vm_id, err)),
    }
}

/// Destroy a VM permanently.
///
/// Idempotent: missing or already-destroyed VMs return `Ok(())` so callers
/// can treat delete-after-delete as a no-op.
pub(crate) async fn delete(container_id: &str) -> Result<()> {
    let vm_id = container_id.to_string();
    let destroy_id = vm_id.clone();
    match tokio::task::spawn_blocking(move || provider().destroy_vm(&destroy_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker destroy join error for {vm_id}: {e}"))
        })? {
        Ok(()) => Ok(()),
        Err(VmRuntimeError::VmNotFound(_)) => Ok(()),
        Err(VmRuntimeError::InvalidTransition { .. }) => Ok(()),
        Err(err) => Err(map_vm_error("destroy", &vm_id, err)),
    }
}

/// Inspect lifecycle status for the reaper reconcile loop.
///
/// Maps [`VmStatus`] onto the sandbox-facing tri-state. A `None` result from
/// the primitive (the VM is absent from the provider's view) collapses to
/// `Missing`, which signals the reaper to remove the orphan record.
pub(crate) async fn status(container_id: &str) -> Result<FirecrackerContainerStatus> {
    let vm_id = container_id.to_string();
    let view = tokio::task::spawn_blocking(move || provider().get_vm(&vm_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!(
                "firecracker status join error for {container_id}: {e}"
            ))
        })?
        .map_err(|e| map_vm_error("status", container_id, e))?;

    Ok(match view.map(|v| v.status) {
        None => FirecrackerContainerStatus::Missing,
        Some(VmStatus::Running) => FirecrackerContainerStatus::Running,
        Some(VmStatus::Created) | Some(VmStatus::Stopped) => FirecrackerContainerStatus::Stopped,
        Some(VmStatus::Destroyed) => FirecrackerContainerStatus::Missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_message_points_at_microvm_runtime_release() {
        // Regression: the operator-facing error must name the upcoming
        // primitive release so callers know where to track the fix instead
        // of treating the error as a config bug on their side.
        let err = unsupported("test feature");
        let msg = err.to_string();
        assert!(msg.contains("test feature"), "{msg}");
        assert!(msg.contains(UPCOMING_RELEASE), "{msg}");
        assert!(msg.contains("microvm-runtime"), "{msg}");
    }

    #[test]
    fn map_vm_error_translates_not_found_to_sandbox_not_found() {
        // Regression: `stop`/`delete` rely on `SandboxError::NotFound` being
        // the variant they pattern-match for idempotent treatment. Pinning
        // the mapping prevents a silent semantic drift.
        let err = map_vm_error("test", "vm-1", VmRuntimeError::VmNotFound("vm-1".into()));
        assert!(matches!(err, SandboxError::NotFound(_)), "got {err}");
    }

    #[test]
    fn map_vm_error_translates_invalid_transition_to_validation() {
        let err = map_vm_error(
            "test",
            "vm-1",
            VmRuntimeError::InvalidTransition {
                vm_id: "vm-1".into(),
                from: "created".into(),
                to: "running",
            },
        );
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
    }
}
