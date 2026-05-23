//! Firecracker microVM lifecycle wrapper.
//!
//! Thin adapter around the [`microvm-runtime`] crate's in-process Firecracker
//! driver. The operator binary **is** the Firecracker host — there is no
//! separate "host-agent" service; this module talks directly to the VMM over
//! its unix socket via the primitive.
//!
//! ## Wired today (`microvm-runtime 0.3.0-alpha.1`)
//!
//! - VM create / start / stop / destroy lifecycle.
//! - **Per-VM TAP / bridge / NAT** via [`NetworkManager`]. The host bridge,
//!   per-VM TAP, and gateway are set up before `create_vm_with_spec`; the
//!   resulting [`VmNetwork`] is recorded so the host-reachable sidecar URL
//!   can be built from the guest IP.
//! - **Per-VM vsock CID + UDS** via [`VsockManager`]. Provisioned pre-boot;
//!   parent dir guaranteed to exist before any `/snapshot/load`.
//! - **Per-VM iptables DNAT** in [`firecracker_dnat`]. Each
//!   `metadata_json.ports` entry installs a PREROUTING DNAT rule mapping
//!   `host_port → guest_ip:container_port`. Rules are tracked per VM and
//!   released on delete.
//! - **Per-VM resource overrides**: `cpu_cores` and `memory_mb` from the
//!   create request flow into `VmSpec` (clamped to FC's u8 / u32 ranges).
//! - **Host-reachable sidecar endpoint URL** computed from the composer-
//!   assigned guest IP and the sidecar port (`SIDECAR_PORT` env, default
//!   8080).
//! - Status reporting for the reaper reconcile loop
//!   (`FirecrackerContainerStatus::{Missing,Running,Stopped}`).
//! - Provider initialization probe used by the operator API health check.
//!
//! ## Still deferred (returns [`SandboxError::Unsupported`] when requested)
//!
//! These need a guest-side handshake we have not built yet. The runtime
//! primitive exposes the channel (vsock), but the guest agent inside the
//! rootfs that consumes it is sandbox-runtime work tracked separately.
//!
//! - **Per-VM environment injection**: requires either cloud-init at boot
//!   or a guest-side metadata service over vsock. The request's `env`
//!   field beyond the runtime envelope (`SIDECAR_PORT`,
//!   `SIDECAR_CAPABILITIES`) cannot reach the guest yet.
//! - **Per-VM disk sizing**: the primitive uses the workspace default
//!   rootfs path; `disk_gb` from the request is accepted for API parity
//!   but ignored. A per-VM ext4 image with the requested size is a
//!   follow-up.
//! - **Sandbox-issued sidecar auth token**: returned as `None`; sealing a
//!   token into the guest requires the vsock metadata service above.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use microvm_runtime::{
    NetworkManager, VmNetwork, VsockManager,
    adapters::firecracker::FirecrackerVmProvider,
    error::VmRuntimeError,
    model::{NetworkInterface, VmSpec, VmStatus, VsockSpec},
    provider::{VmProvider, VmQuery},
};

use crate::error::{Result, SandboxError};

use crate::firecracker_dnat;

/// Single process-wide handle to the Firecracker provider.
///
/// The provider is `Clone`able but keeps an internal `Arc<RwLock<…>>` of VM
/// records, so we only want one instance per operator process — multiple
/// instances would diverge state and leak Firecracker child processes.
fn provider() -> &'static FirecrackerVmProvider {
    static PROVIDER: OnceLock<FirecrackerVmProvider> = OnceLock::new();
    PROVIDER.get_or_init(FirecrackerVmProvider::from_env)
}

/// Process-wide [`NetworkManager`]. The manager is stateless (the kernel is
/// the source of truth for bridge / TAP / iptables), so sharing one instance
/// across the process is safe and avoids re-reading env vars per call.
fn network() -> &'static NetworkManager {
    static NETWORK: OnceLock<NetworkManager> = OnceLock::new();
    NETWORK.get_or_init(NetworkManager::from_env)
}

/// Process-wide [`VsockManager`]. Unlike [`network`], the vsock manager owns
/// an in-process CID allocation map, so all `attach`/`detach` calls must go
/// through the same instance to avoid CID double-allocation.
fn vsock_manager() -> &'static VsockManager {
    static VSOCK: OnceLock<VsockManager> = OnceLock::new();
    VSOCK.get_or_init(VsockManager::from_env)
}

/// Per-VM bookkeeping captured at create time so `delete` can release the
/// composer-managed host resources without having to re-derive them.
///
/// Stored as the value side of [`ATTACHMENTS`]; keyed by `vm_id`. Network /
/// vsock are released via their managers; DNAT rules are released by chain
/// name via [`firecracker_dnat::release_port_forwards`].
#[derive(Debug, Clone)]
struct VmAttachments {
    network_attached: bool,
    vsock_attached: bool,
    /// Number of installed DNAT rules. Used as a tombstone — non-zero means
    /// we created at least one rule and the per-VM chain must be torn down.
    dnat_rule_count: usize,
}

fn attachments_map() -> &'static Mutex<HashMap<String, Arc<VmAttachments>>> {
    static MAP: OnceLock<Mutex<HashMap<String, Arc<VmAttachments>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_attachments(vm_id: &str, value: VmAttachments) {
    if let Ok(mut guard) = attachments_map().lock() {
        guard.insert(vm_id.to_string(), Arc::new(value));
    }
}

fn take_attachments(vm_id: &str) -> Option<Arc<VmAttachments>> {
    attachments_map().lock().ok()?.remove(vm_id)
}

/// Sidecar HTTP port the guest listens on. The same value is also baked
/// into the guest's `SIDECAR_PORT` env var by the runtime layer; reading
/// it here keeps the URL construction colocated with the FC wiring.
fn sidecar_port() -> u16 {
    std::env::var("SIDECAR_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080)
}

fn unsupported(feature: &str) -> SandboxError {
    SandboxError::Unsupported(format!(
        "{feature} is not yet supported by the in-process Firecracker driver; \
         a vsock-backed guest metadata service is the next milestone. \
         See https://github.com/tangle-network/microvm-runtime"
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
        VmRuntimeError::SnapshotNotFound { snapshot_id, .. } => SandboxError::NotFound(format!(
            "{action} vm {vm_id}: snapshot {snapshot_id} not found"
        )),
        VmRuntimeError::StatePoisoned => SandboxError::Unavailable(format!(
            "{action} vm {vm_id}: microvm-runtime state lock poisoned"
        )),
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
        | VmRuntimeError::Uffd(msg) => {
            SandboxError::Unavailable(format!("{action} vm {vm_id}: microvm-runtime: {msg}"))
        }
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
/// `id` is the `vm_id` passed to [`microvm-runtime`]. `endpoint` is the
/// host-reachable sidecar URL built from the composer-assigned guest IP.
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerContainer {
    pub id: String,
    pub endpoint: Option<String>,
}

/// Sandbox-side view of a successful provision call.
///
/// `sidecar_auth_token` stays `None` until the vsock-backed metadata
/// service lands; until then the runtime layer falls back to a sandbox-
/// generated token (not sealed into the guest).
#[derive(Clone, Debug)]
pub(crate) struct FirecrackerProvisionResult {
    pub container: FirecrackerContainer,
    pub sidecar_auth_token: Option<String>,
}

/// Sandbox-side create request.
///
/// `image` and `disk_gb` are accepted for API parity but currently ignored
/// — the primitive uses the workspace default rootfs path and size. `env`
/// beyond the runtime-injected envelope keys cannot reach the guest yet
/// (see module docs).
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

/// Build the per-VM [`VmSpec`] from request-level resource overrides.
///
/// Pre-composition fields only; the [`NetworkInterface`] and [`VsockSpec`]
/// are appended by `attach_network_and_vsock` once they have been allocated.
fn spec_from_request(req: &FirecrackerCreateRequest) -> VmSpec {
    let mut spec = VmSpec::default();
    if req.cpu_cores > 0 {
        // `cpu_cores` is `u64` on the sandbox side, but FC's `vcpu_count` is
        // `u8` (the kernel's max is 255). Clamp rather than overflow-panic.
        spec.vcpu_count = Some(req.cpu_cores.min(u8::MAX as u64) as u8);
    }
    if req.memory_mb > 0 {
        spec.mem_size_mib = Some(req.memory_mb.min(u32::MAX as u64) as u32);
    }
    spec
}

/// Allocate the host TAP + vsock CID for `vm_id`, augmenting `spec` with the
/// resulting network interface and vsock binding. Returns the [`VmNetwork`]
/// so the caller can build the host-reachable endpoint URL.
///
/// This deliberately does NOT use [`microvm_runtime::FirecrackerComposer`]:
/// the composer hides the per-VM addressing from the caller (the `VmSpec`
/// it mutates is consumed inside the provider and never returned), and we
/// need the guest IP to build the endpoint URL.
fn attach_network_and_vsock(vm_id: &str, spec: &mut VmSpec) -> Result<VmNetwork> {
    let net = network();
    net.ensure_host()
        .map_err(|e| map_vm_error("ensure_host", vm_id, e))?;
    let vm_net = net
        .attach(vm_id)
        .map_err(|e| map_vm_error("network_attach", vm_id, e))?;
    let guest_mac = vm_net.mac_string();
    spec.network_interfaces.push(NetworkInterface {
        iface_id: "eth0".into(),
        host_dev_name: vm_net.tap_name.clone(),
        guest_mac: Some(guest_mac),
        rx_rate_limiter: None,
        tx_rate_limiter: None,
    });

    let vsock = vsock_manager();
    let vm_vsock = vsock
        .attach(vm_id)
        .map_err(|e| map_vm_error("vsock_attach", vm_id, e))?;
    vsock
        .ensure_uds_parent(&vm_vsock.uds_path)
        .map_err(|e| map_vm_error("vsock_ensure_uds_parent", vm_id, e))?;
    spec.vsock = Some(VsockSpec {
        cid: vm_vsock.cid,
        uds_path: vm_vsock.uds_path,
    });

    Ok(vm_net)
}

/// Tear down the host-side allocations associated with `vm_id`. Best-effort:
/// individual failures are logged via `tracing` but never propagated. Used
/// both on the create-failure cleanup path and on `delete`.
fn release_attachments(vm_id: &str, attachments: &VmAttachments) {
    if attachments.dnat_rule_count > 0
        && let Err(err) = firecracker_dnat::release_port_forwards(vm_id)
    {
        tracing::warn!(vm_id, %err, "failed to release firecracker DNAT rules");
    }
    if attachments.vsock_attached
        && let Err(err) = vsock_manager().detach(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to detach firecracker vsock");
    }
    if attachments.network_attached
        && let Err(err) = network().detach(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to detach firecracker network");
    }
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

/// Create a VM and start it via the in-process driver, attaching network,
/// vsock, and any requested host port forwards.
///
/// On any failure after VM creation, the VM is destroyed and all attached
/// host resources are released before returning. On success, a record of
/// the attachments is stored so a later `delete` releases them.
pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    // Per-VM environment injection still requires a guest metadata service.
    // Fail loudly rather than silently dropping caller-supplied env entries.
    let injected_keys = ["SIDECAR_PORT", "SIDECAR_CAPABILITIES"];
    let has_user_env = req.env.keys().any(|k| !injected_keys.contains(&k.as_str()));
    if has_user_env {
        return Err(unsupported(
            "per-VM environment injection for firecracker sandboxes",
        ));
    }

    let vm_id = req.session_id.clone();
    let mut spec = spec_from_request(&req);

    // Compose the per-VM TAP + vsock binding pre-spawn. The returned
    // `VmNetwork` carries the guest IP that the sidecar endpoint URL is
    // built from.
    let vm_net = match attach_network_and_vsock(&vm_id, &mut spec) {
        Ok(v) => v,
        Err(err) => {
            // ensure_host failed, or attach failed mid-way — release whatever
            // partial state may have been created. Both managers are
            // idempotent under "nothing to release".
            let _ = network().detach(&vm_id);
            let _ = vsock_manager().detach(&vm_id);
            return Err(err);
        }
    };

    // The blocking lifecycle calls on `FirecrackerVmProvider` shell out to a
    // child process and do unix-socket I/O; isolate them onto a blocking
    // worker so the async runtime stays responsive.
    let create_id = vm_id.clone();
    let create_spec = spec.clone();
    if let Err(err) = tokio::task::spawn_blocking(move || {
        provider().create_vm_with_spec(&create_id, &create_spec)
    })
    .await
    .map_err(|e| {
        SandboxError::Unavailable(format!("firecracker create join error for {vm_id}: {e}"))
    })?
    .map_err(|e| map_vm_error("create", &vm_id, e))
    {
        release_attachments(
            &vm_id,
            &VmAttachments {
                network_attached: true,
                vsock_attached: true,
                dnat_rule_count: 0,
            },
        );
        return Err(err);
    }

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
        release_attachments(
            &vm_id,
            &VmAttachments {
                network_attached: true,
                vsock_attached: true,
                dnat_rule_count: 0,
            },
        );
        return Err(err);
    }

    // Install DNAT for each requested port forward. The host doesn't know
    // its own public IP in this context — we DNAT on the wildcard host
    // address and let the kernel match traffic on the egress interface.
    let mut dnat_rule_count = 0usize;
    for port in &req.ports {
        if let Err(err) = firecracker_dnat::install_port_forward(&vm_id, vm_net.guest_ip, *port) {
            tracing::error!(
                vm_id = %vm_id,
                host_port = port.host_port,
                container_port = port.container_port,
                %err,
                "failed to install DNAT for firecracker port forward; tearing down"
            );
            // Tear everything down.
            let cleanup_id = vm_id.clone();
            let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
            release_attachments(
                &vm_id,
                &VmAttachments {
                    network_attached: true,
                    vsock_attached: true,
                    dnat_rule_count,
                },
            );
            return Err(SandboxError::Unavailable(format!(
                "firecracker port forward install failed for {vm_id}: {err}"
            )));
        }
        dnat_rule_count += 1;
    }

    record_attachments(
        &vm_id,
        VmAttachments {
            network_attached: true,
            vsock_attached: true,
            dnat_rule_count,
        },
    );

    let endpoint = format!("http://{}:{}", vm_net.guest_ip, sidecar_port());

    Ok(FirecrackerProvisionResult {
        container: FirecrackerContainer {
            id: vm_id,
            endpoint: Some(endpoint),
        },
        // Vsock-injected token requires a guest-side metadata agent the
        // sandbox rootfs does not ship yet. Until that lands the runtime
        // layer generates its own token and treats the sidecar as unsealed.
        sidecar_auth_token: None,
    })
}

/// Resume a previously-stopped VM.
///
/// Used by the sandbox `resume` lifecycle. The host attachments (TAP, CID,
/// DNAT) survive across stop/start, so resuming a known VM only needs to
/// drive the primitive's lifecycle and rebuild the endpoint from the
/// recorded `VmNetwork`-equivalent allocation.
pub(crate) async fn start(container_id: &str) -> Result<FirecrackerContainer> {
    let vm_id = container_id.to_string();
    let start_id = vm_id.clone();
    tokio::task::spawn_blocking(move || provider().start_vm(&start_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker start join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("start", &vm_id, e))?;

    // `NetworkManager::attach` is idempotent: for a known vm_id it returns
    // the same TAP / IP allocation. Resuming a VM whose TAP was torn down
    // out-of-band recreates it; this matches what an operator would do by
    // hand for debugging.
    let vm_net = network()
        .attach(container_id)
        .map_err(|e| map_vm_error("network_attach_resume", container_id, e))?;
    let endpoint = format!("http://{}:{}", vm_net.guest_ip, sidecar_port());

    Ok(FirecrackerContainer {
        id: container_id.to_string(),
        endpoint: Some(endpoint),
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

/// Destroy a VM permanently and release all per-VM host attachments
/// (DNAT rules, vsock CID, TAP).
///
/// Idempotent: missing or already-destroyed VMs return `Ok(())` so callers
/// can treat delete-after-delete as a no-op. Attachment release runs even
/// if the VMM tear-down errors out — leaving the host with orphan iptables
/// rules or TAPs is worse than swallowing a redundant destroy error.
pub(crate) async fn delete(container_id: &str) -> Result<()> {
    let vm_id = container_id.to_string();

    let destroy_id = vm_id.clone();
    let destroy_outcome = tokio::task::spawn_blocking(move || provider().destroy_vm(&destroy_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker destroy join error for {vm_id}: {e}"))
        })?;

    // Release host-side allocations whether or not destroy succeeded — the
    // VM is going away either way and orphan resources are worse than
    // double-released ones (every release path is idempotent).
    if let Some(attachments) = take_attachments(&vm_id) {
        release_attachments(&vm_id, &attachments);
    } else {
        // Best-effort release for VMs not in our map (e.g. restart of the
        // operator process lost the in-memory record).
        if let Err(err) = firecracker_dnat::release_port_forwards(&vm_id) {
            tracing::debug!(vm_id = %vm_id, %err, "release_port_forwards on unknown vm_id");
        }
        let _ = vsock_manager().detach(&vm_id);
        let _ = network().detach(&vm_id);
    }

    match destroy_outcome {
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
    fn unsupported_message_points_at_microvm_runtime() {
        // Regression: the operator-facing error must explain which deferred
        // capability tripped and where the fix lands, instead of looking
        // like a config bug on the caller's side.
        let err = unsupported("test feature");
        let msg = err.to_string();
        assert!(msg.contains("test feature"), "{msg}");
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

    #[test]
    fn attachments_roundtrip_is_take_once() {
        // Regression: a second `delete` after the first must not see the
        // attachments again (otherwise we'd double-release DNAT rules,
        // which is fine, but also issue a misleading WARN log).
        let vm_id = "vm-attach-roundtrip";
        // Ensure no leftover state from a previous test run.
        let _ = take_attachments(vm_id);
        record_attachments(
            vm_id,
            VmAttachments {
                network_attached: true,
                vsock_attached: true,
                dnat_rule_count: 2,
            },
        );
        let first = take_attachments(vm_id).expect("first take returns recorded value");
        assert_eq!(first.dnat_rule_count, 2);
        assert!(take_attachments(vm_id).is_none());
    }
}
