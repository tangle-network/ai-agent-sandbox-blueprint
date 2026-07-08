//! VmRuntimeError → SandboxError translation.

use super::*;

/// Secret name used by the guest metadata daemon for the host-issued sidecar
/// auth token. The guest stack convention is for the sidecar process to read
/// this from the per-secret file the daemon writes (default
/// `/var/run/microvm-guest/secrets/sidecar_auth_token`).
pub(crate) const SIDECAR_AUTH_TOKEN_SECRET: &str = "sidecar_auth_token";

pub(crate) fn map_vm_error(action: &str, vm_id: &str, err: VmRuntimeError) -> SandboxError {
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
        VmRuntimeError::SnapshotNotFound { snapshot_id, .. } => SandboxError::NotFound(format!(
            "{action} vm {vm_id}: snapshot {snapshot_id} not found"
        )),
        VmRuntimeError::StatePoisoned => SandboxError::Unavailable(format!(
            "{action} vm {vm_id}: microvm-runtime state lock poisoned"
        )),
        // `Unsupported` here comes from the primitive (e.g. backend feature
        // gate), not from us — surface it as `Unavailable` so callers retry
        // by re-checking host config rather than treating it as a hard
        // "feature missing" claim against the sandbox API.
        VmRuntimeError::Unsupported(msg) => SandboxError::Unavailable(format!(
            "{action} vm {vm_id}: firecracker backend not ready: {msg}"
        )),
        VmRuntimeError::Metrics(msg)
        | VmRuntimeError::Shutdown(msg)
        | VmRuntimeError::Firewall(msg)
        | VmRuntimeError::Jailer(msg)
        | VmRuntimeError::NetworkConfig(msg)
        | VmRuntimeError::NetworkSetup(msg)
        | VmRuntimeError::Rootfs(msg)
        | VmRuntimeError::Uffd(msg)
        | VmRuntimeError::GuestMetadata(msg) => {
            SandboxError::Unavailable(format!("{action} vm {vm_id}: microvm-runtime: {msg}"))
        }
    }
}
