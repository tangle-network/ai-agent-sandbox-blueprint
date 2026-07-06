//! Firecracker container status + create/provision request-response types.

use super::*;

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
/// `id` is the `vm_id` passed to [`microvm-runtime`]. `endpoint` is the
/// host-reachable sidecar URL built from the composer-assigned guest IP.
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerContainer {
    pub id: String,
    pub endpoint: Option<String>,
}

/// Sandbox-side view of a successful provision call.
///
/// `sidecar_auth_token` carries the 32-byte token the host minted and pushed
/// into the guest via the metadata service. The runtime layer stamps it onto
/// the sandbox record so subsequent sidecar calls authenticate against the
/// same value the guest stored.
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerProvisionResult {
    pub container: FirecrackerContainer,
    pub sidecar_auth_token: Option<String>,
}

/// Sandbox-side create request.
///
/// `image` is the stack name (e.g. `"node-20"`). `disk_gb`, when non-zero,
/// resizes the per-VM rootfs clone via
/// [`RootfsRegistry::clone_for_vm_with_size`]. `env` is pushed verbatim into
/// the guest by the metadata service after boot — both runtime-injected
/// envelope keys and caller-supplied keys flow through the same path.
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
