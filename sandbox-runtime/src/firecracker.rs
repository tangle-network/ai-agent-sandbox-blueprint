//! Firecracker microVM lifecycle wrapper.
//!
//! Thin adapter around the [`microvm-runtime`] crate's in-process Firecracker
//! driver. The operator binary **is** the Firecracker host — there is no
//! separate "host-agent" service; this module talks directly to the VMM over
//! its unix socket via the primitive.
//!
//! ## Wired today (`microvm-runtime 0.4.0-alpha.2` + `microvm-warm-pool 0.1.0-alpha.2`)
//!
//! - VM create / start / stop / destroy lifecycle.
//! - **Warm-pool snapshot serving** via [`crate::firecracker_warm`], gated
//!   by `SANDBOX_FC_WARM_POOL_SIZE` (0 = off, the default). When enabled,
//!   dedicated template VMs (never tenant VMs) are cold-booted, paused, and
//!   snapshotted; pre-restored entries wait in a [`microvm_warm_pool`] pool
//!   and are handed off to creates via `rename_vm` + resume. Misses fall
//!   back to cold boot with a logged, typed reason — the one designed
//!   fallback in this module.
//! - **Snapshot-restore memory backend**: `MICROVM_MEM_BACKEND=file|uffd`
//!   is read by the primitive itself (`FirecrackerConfig::from_env`);
//!   `uffd` restores guest memory through a userfaultfd handler (lazy CoW
//!   paging from the snapshot's mem file) instead of a full file load.
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
//! - **Per-VM disk sizing**: when `req.disk_gb > 0` the request's chosen
//!   stack is cloned through [`RootfsRegistry::clone_for_vm_with_size`] and
//!   the resulting per-VM ext4 image is wired into [`VmSpec::rootfs`]. The
//!   default stack name comes from `SANDBOX_FIRECRACKER_DEFAULT_STACK` when
//!   `req.image` is empty; when both are absent the workspace default
//!   rootfs path baked into the provider is reused untouched.
//! - **Per-VM environment + sidecar auth token injection** via the guest
//!   metadata service ([`GuestMetadataClient`]). Post-boot, the host opens
//!   the per-VM vsock UDS and pushes the full `req.env` map plus a freshly
//!   minted 32-byte sidecar auth token into the guest. The token is also
//!   returned to the caller so the runtime layer can stamp it onto the
//!   sandbox record.
//! - **Host-reachable sidecar endpoint URL** computed from the composer-
//!   assigned guest IP and the sidecar port (`SIDECAR_PORT` env, default
//!   8080).
//! - Status reporting for the reaper reconcile loop
//!   (`FirecrackerContainerStatus::{Missing,Running,Stopped}`).
//! - Provider initialization probe used by the operator API health check.
//!
//! ## Operator prerequisites
//!
//! - A guest-side metadata daemon listening on vsock port
//!   `MICROVM_GUEST_METADATA_PORT` (default `5555`) baked into the rootfs.
//!   The reference implementation ships at
//!   `microvm-runtime/examples/guest_metadata_daemon.rs`; operators should
//!   install it as a systemd unit (or equivalent) inside their stack image.
//! - Stack templates under `MICROVM_ROOTFS_TEMPLATE_DIR` with per-VM
//!   clones written to `MICROVM_ROOTFS_CLONES_DIR`. The default stack name
//!   used when the create request leaves `image` empty is configured via
//!   `SANDBOX_FIRECRACKER_DEFAULT_STACK`.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use microvm_runtime::{
    GuestMetadataClient, GuestMetadataConfig, NetworkManager, RootfsRegistry, VmNetwork,
    VsockManager,
    adapters::firecracker::{FirecrackerConfig, FirecrackerVmProvider},
    error::VmRuntimeError,
    model::{NetworkInterface, VmSpec, VmStatus, VsockSpec},
    provider::{VmProvider, VmQuery},
};
use rand::RngCore;
use rand::rngs::OsRng;

use crate::error::{Result, SandboxError};
use crate::firecracker_warm::{
    self, TemplateIdentity, WarmClaim, WarmClaimRequest, WarmHost, WarmLineage, WarmOutcome,
    WarmServing, WarmSettings,
};

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

/// Process-wide [`RootfsRegistry`]. The registry only caches `(path, mtime)
/// → sha256`, which is safe to share; the per-VM clone slots it produces are
/// keyed by `vm_id` so callers cannot collide across sandboxes.
fn rootfs_registry() -> &'static RootfsRegistry {
    static REGISTRY: OnceLock<RootfsRegistry> = OnceLock::new();
    REGISTRY.get_or_init(RootfsRegistry::from_env)
}

/// Per-VM bookkeeping captured at create time so `delete` can release the
/// composer-managed host resources without having to re-derive them.
///
/// Stored as the value side of [`ATTACHMENTS`]; keyed by `vm_id`. Network /
/// vsock are released via their managers; DNAT rules are released by chain
/// name via [`firecracker_dnat::release_port_forwards`]; rootfs clones are
/// released via [`RootfsRegistry::release`] when [`rootfs_released`] is set.
#[derive(Debug, Clone)]
struct VmAttachments {
    network_attached: bool,
    vsock_attached: bool,
    /// Number of installed DNAT rules. Used as a tombstone — non-zero means
    /// we created at least one rule and the per-VM chain must be torn down.
    dnat_rule_count: usize,
    /// `true` iff a per-VM rootfs clone was created and must be released
    /// on delete. `false` for VMs that reused the provider's default rootfs.
    rootfs_cloned: bool,
    /// Warm-claim lineage: host resources this sandbox holds under ids other
    /// than its own (rider TAP, template vsock CID, template rootfs clone).
    /// `None` for cold-booted VMs. This in-memory copy serves the normal
    /// delete; a durable copy is persisted by [`crate::firecracker_lineage`] so
    /// a delete or reconcile after an operator restart still releases the
    /// aliases instead of leaking them.
    warm: Option<WarmLineage>,
}

impl VmAttachments {
    fn cold(dnat_rule_count: usize, rootfs_cloned: bool) -> Self {
        Self {
            network_attached: true,
            vsock_attached: true,
            dnat_rule_count,
            rootfs_cloned,
            warm: None,
        }
    }
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

/// Stack name used when `req.image` is empty. `None` means "fall back to
/// the workspace default rootfs path the provider was constructed with" —
/// no per-VM clone is performed in that case.
fn default_stack_name() -> Option<String> {
    std::env::var("SANDBOX_FIRECRACKER_DEFAULT_STACK")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Secret name used by the guest metadata daemon for the host-issued sidecar
/// auth token. The guest stack convention is for the sidecar process to read
/// this from the per-secret file the daemon writes (default
/// `/var/run/microvm-guest/secrets/sidecar_auth_token`).
const SIDECAR_AUTH_TOKEN_SECRET: &str = "sidecar_auth_token";

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

/// Build the per-VM [`VmSpec`] from request-level resource overrides.
///
/// Pre-composition fields only; the [`NetworkInterface`] and [`VsockSpec`]
/// are appended by `attach_network_and_vsock` once they have been allocated,
/// and the optional per-VM rootfs path is set by `attach_rootfs` when a disk
/// resize is requested.
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
/// so the caller can build the host-reachable endpoint URL and the
/// per-VM UDS path so the post-boot metadata client can dial the guest.
///
/// This deliberately does NOT use [`microvm_runtime::FirecrackerComposer`]:
/// the composer hides the per-VM addressing from the caller (the `VmSpec`
/// it mutates is consumed inside the provider and never returned), and we
/// need the guest IP to build the endpoint URL.
fn attach_network_and_vsock(
    vm_id: &str,
    spec: &mut VmSpec,
) -> Result<(VmNetwork, std::path::PathBuf)> {
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
    let uds_path = vm_vsock.uds_path.clone();
    spec.vsock = Some(VsockSpec {
        cid: vm_vsock.cid,
        uds_path: vm_vsock.uds_path,
    });

    Ok((vm_net, uds_path))
}

/// Resolve the requested stack name and, when one applies, clone it into a
/// per-VM rootfs slot sized to `disk_gb`. Returns `Ok(true)` when a clone
/// was performed (so `delete` knows to release it), `Ok(false)` when the
/// provider's default rootfs path was reused.
///
/// `disk_gb == 0` keeps the provider default regardless of `image`, matching
/// the historical behaviour where `disk_gb` was accepted for API parity.
fn attach_rootfs(vm_id: &str, req: &FirecrackerCreateRequest, spec: &mut VmSpec) -> Result<bool> {
    if req.disk_gb == 0 {
        return Ok(false);
    }
    let stack_name = if req.image.trim().is_empty() {
        match default_stack_name() {
            Some(s) => s,
            // No stack to clone from. Keep the provider default rootfs; the
            // caller asked for a size override we cannot honour without a
            // template, but failing the create here would be worse than
            // surfacing the (still-functional) default-size VM.
            None => return Ok(false),
        }
    } else {
        req.image.trim().to_string()
    };

    let target_bytes = req.disk_gb.saturating_mul(1024 * 1024 * 1024);
    let registry = rootfs_registry();
    let rootfs = registry
        .clone_for_vm_with_size(vm_id, &stack_name, target_bytes)
        .map_err(|e| map_vm_error("rootfs_clone", vm_id, e))?;
    spec.rootfs = Some(rootfs.path);
    Ok(true)
}

/// Mint a 32-byte sidecar auth token, URL-safe base64-encoded.
///
/// The token is opaque to the host past this point — it is pushed verbatim
/// into the guest secrets directory and returned to the caller so the
/// sandbox record can stamp the same value the sidecar will compare against.
fn mint_sidecar_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    // Match `auth::generate_token`'s hex encoding so downstream consumers
    // (sidecar comparators, log redactors) see a single token format.
    hex::encode(bytes)
}

/// Push the env table and the sidecar auth token into the guest via the
/// metadata service. Returns the minted token on success.
///
/// The connection takes a moment to come up after `start_vm` (cold boot is
/// 1-3s, snapshot restore <100ms); [`GuestMetadataClient::connect`] handles
/// the retry loop against `connect_timeout`. The whole transaction runs on
/// a blocking worker because the underlying UnixStream calls block.
async fn inject_runtime_metadata(
    vm_id: &str,
    uds_path: std::path::PathBuf,
    env: HashMap<String, String>,
) -> Result<String> {
    let token = mint_sidecar_token();
    let token_bytes = token.clone().into_bytes();
    let vm_id_owned = vm_id.to_string();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<(), VmRuntimeError> {
        let client = GuestMetadataClient::new(uds_path, GuestMetadataConfig::from_env());
        let mut conn = client.connect()?;
        if !env.is_empty() {
            conn.set_env(&env)?;
        }
        conn.set_secret(SIDECAR_AUTH_TOKEN_SECRET, &token_bytes)?;
        Ok(())
    })
    .await
    .map_err(|e| {
        SandboxError::Unavailable(format!("guest metadata join error for {vm_id_owned}: {e}"))
    })?;
    result.map_err(|e| map_vm_error("guest_metadata", vm_id, e))?;
    Ok(token)
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
    if attachments.rootfs_cloned
        && let Err(err) = rootfs_registry().release(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to release firecracker rootfs clone");
    }
    if let Some(warm) = &attachments.warm {
        // Warm-claimed sandboxes hold host resources under lineage ids: the
        // vsock CID (and, when cloned, the rootfs slot) live under the
        // template id; the TAP the VM rides lives under the rider id. All
        // are single-owner by construction (one claim per generation), so
        // releasing them here cannot race another VM.
        if warm.rootfs_cloned
            && let Err(err) = rootfs_registry().release(&warm.template_id)
        {
            tracing::warn!(
                vm_id,
                template_id = %warm.template_id,
                ?err,
                "failed to release warm template rootfs clone"
            );
        }
        if let Err(err) = vsock_manager().detach(&warm.template_id) {
            tracing::warn!(
                vm_id,
                template_id = %warm.template_id,
                ?err,
                "failed to detach warm template vsock"
            );
        }
        if let Some(rider_id) = &warm.rider_id
            && let Err(err) = network().detach(rider_id)
        {
            tracing::warn!(vm_id, rider_id, ?err, "failed to detach warm rider network");
        }
    }
}

/// A warm-pool Firecracker process orphaned by a previous operator process.
struct WarmOrphan {
    pid: i32,
    vm_id: String,
}

/// Reap warm-pool Firecracker processes orphaned by a previous operator
/// process. The pool is process-local: on restart the fresh process's provider
/// maps are empty, so prior `fcwarm-*` templates and `warm-*` pool entries keep
/// running — holding guest memory — with no sandbox record and no in-memory
/// handle to stop them. This finds them from the OS, SIGKILLs them, and
/// releases their by-id host resources.
///
/// MUST run before the first `seed_generation`: a fresh process resets the
/// generation counter to 0 and re-mints the exact same ids, so reaping after a
/// seed would kill the just-seeded generation. Called as the first step of the
/// `WARM_SERVING` init closure, and from `reaper::reconcile_on_startup` (which
/// also covers the "warm disabled now, but a prior process left orphans" case
/// the lazy engine init never reaches).
pub(crate) fn reconcile_warm_orphans() {
    let socket_dir = provider().config.socket_dir.clone();
    // Live sandboxes are never warm-prefixed, so the prefix filter already
    // spares them; skipping stored ids too is belt-and-suspenders.
    let live: std::collections::HashSet<String> = crate::runtime::sandboxes()
        .and_then(|s| s.values())
        .map(|recs| {
            recs.into_iter()
                .flat_map(|r| [r.id, r.container_id])
                .collect()
        })
        .unwrap_or_default();

    for orphan in enumerate_warm_fc_processes(&socket_dir) {
        if live.contains(&orphan.vm_id) {
            continue;
        }
        tracing::warn!(
            pid = orphan.pid,
            vm_id = %orphan.vm_id,
            "reaping orphaned warm-pool firecracker process from a previous operator process"
        );
        reap_pid(orphan.pid);
        release_orphan_resources(&orphan.vm_id);
    }

    reclaim_orphan_rootfs_clones();
}

/// Reclaim leaked warm-template rootfs clones after the process reap.
///
/// A cold sandbox's clone lives under its own id and is released by
/// `delete`'s own-id path even after a restart, so it never leaks. Only the
/// aliased warm *template* clone (`fcwarm-*` / `warm-*`) can be orphaned — its
/// template process is destroyed at claim, so the `/proc` scan never sees it,
/// and if the owning sandbox's delete didn't run (crash, or a failed lineage
/// persist) its clone is stranded. Reclaim a warm-prefixed clone dir iff no
/// live sandbox's persisted lineage still references it.
///
/// Scoping to warm-prefixed dirs is the data-loss guard: a live sandbox's
/// own-id clone is never warm-prefixed, so this can never delete a live
/// workspace out from under a running sandbox.
fn reclaim_orphan_rootfs_clones() {
    let referenced: std::collections::HashSet<String> =
        crate::firecracker_lineage::referenced_template_ids()
            .into_iter()
            .collect();
    let clones_dir = rootfs_registry().config().clones_dir.clone();
    let Ok(entries) = std::fs::read_dir(&clones_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if should_reclaim_clone(&id, &referenced) {
            tracing::warn!(
                clone_id = %id,
                "reclaiming orphaned warm-template rootfs clone (no live sandbox lineage references it)"
            );
            let _ = rootfs_registry().release(&id);
        }
    }
}

/// A `clones_dir` entry is reclaimable iff it is a warm-template clone
/// (`fcwarm-*` / `warm-*`) that no live sandbox's lineage still references. A
/// non-warm-prefixed id is NEVER reclaimed — that is a live sandbox's own-id
/// clone, and removing it would destroy a running workspace. This predicate is
/// the data-loss guard for the sweep.
fn should_reclaim_clone(id: &str, referenced: &std::collections::HashSet<String>) -> bool {
    is_warm_vm_id(id) && !referenced.contains(id)
}

/// Enumerate live Firecracker processes whose API socket sits under our
/// `socket_dir` and whose vm_id carries a warm-pool prefix. Reads only `/proc`
/// — the provider's in-memory maps are empty in a freshly started process, so a
/// prior process's VMs are invisible to it and must be found from the OS.
fn enumerate_warm_fc_processes(socket_dir: &std::path::Path) -> Vec<WarmOrphan> {
    let mut orphans = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return orphans;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        // cmdline is NUL-separated argv.
        let args: Vec<&str> = cmdline
            .split(|b| *b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .filter(|s| !s.is_empty())
            .collect();
        let Some(sock) = args
            .iter()
            .position(|a| *a == "--api-sock")
            .and_then(|i| args.get(i + 1))
        else {
            continue;
        };
        // Socket layout is `<socket_dir>/<vm_id>/api.sock`; require the socket
        // to live under OUR socket_dir so we never touch an unrelated VMM.
        let sock_path = std::path::Path::new(sock);
        if sock_path.parent().and_then(|p| p.parent()) != Some(socket_dir) {
            continue;
        }
        let Some(vm_id) = sock_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        else {
            continue;
        };
        // A warm CLAIM (`rename_vm`) moves the socket dir on disk but cannot
        // rewrite a running FC's `--api-sock` argv, so a LIVE claimed sandbox
        // still shows the warm-entry path in `/proc/cmdline`. Its socket file
        // has moved and no longer exists at that path; only an unclaimed orphan
        // still has its socket there. Skipping the missing-socket case is what
        // prevents reconcile from SIGKILLing a live warm-claimed sandbox after
        // an operator restart — data loss, not a leak.
        if is_warm_vm_id(vm_id) && sock_path.exists() {
            orphans.push(WarmOrphan {
                pid,
                vm_id: vm_id.to_string(),
            });
        }
    }
    orphans
}

/// A warm-pool vm_id: `fcwarm-*` templates/riders or `warm-*` pool entries
/// (`microvm-warm-pool` names entries `warm-<stack>-<ver>-<seed>`). Production
/// sandbox ids are session UUIDs and never carry these prefixes, so this is the
/// primary guard against reaping a live sandbox.
fn is_warm_vm_id(vm_id: &str) -> bool {
    vm_id.starts_with("fcwarm-") || vm_id.starts_with("warm-")
}

/// SIGKILL a pid via the `kill` utility. The orphan was reparented to init when
/// its parent (the previous operator process) exited, so it is not our child
/// and cannot be `waitpid`-ed; init reaps the zombie. `libc` is gated behind a
/// TEE feature in this crate and the Firecracker path is Linux-only, so shell
/// out rather than widen the default dependency set.
fn reap_pid(pid: i32) {
    match std::process::Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            tracing::warn!(
                pid,
                ?status,
                "kill -KILL of orphaned firecracker returned non-zero"
            )
        }
        Err(err) => tracing::warn!(pid, %err, "failed to signal orphaned firecracker process"),
    }
}

/// Release the by-id host resources an orphaned warm VM held. Every manager
/// derives its kernel/host object from the vm_id (TAP = `tap-<hash(id)>`, vsock
/// UDS = `uds_path_for(id)`, rootfs clone = `clones_dir/id`, DNAT chain =
/// `hash(id)`), so release works without the in-memory record a fresh process
/// lacks. All detaches are best-effort and idempotent.
fn release_orphan_resources(vm_id: &str) {
    release_attachments(
        vm_id,
        &VmAttachments {
            network_attached: true,
            vsock_attached: true,
            dnat_rule_count: 1,
            rootfs_cloned: true,
            warm: None,
        },
    );
    // A template (`fcwarm-g<N>-tpl`) rides a sibling rider TAP under
    // `fcwarm-g<N>-rider`; that host interface has no FC process of its own, so
    // enumeration never sees it — derive and detach it here.
    if let Some(base) = vm_id.strip_suffix("-tpl") {
        let rider_id = format!("{base}-rider");
        if let Err(err) = network().detach(&rider_id) {
            tracing::warn!(
                vm_id,
                rider_id,
                ?err,
                "failed to detach orphaned warm rider TAP"
            );
        }
    }
    // On-disk residue: API socket dir + persisted vmstate dir.
    let _ = std::fs::remove_dir_all(provider().config.socket_dir.join(vm_id));
    let _ = std::fs::remove_dir_all(provider().vm_state_path(vm_id));
}

/// Production [`WarmHost`]: composes template identity with the same
/// primitives the cold path uses (`NetworkManager` / `VsockManager` /
/// `RootfsRegistry`) and gates template readiness on the guest metadata
/// daemon answering over vsock — the identical readiness bar a cold-booted
/// sandbox must clear before env injection.
struct OperatorWarmHost;

#[async_trait]
impl WarmHost for OperatorWarmHost {
    async fn compose_template(
        &self,
        template_id: &str,
        disk_gb: u64,
        stack: Option<&str>,
        spec: &mut VmSpec,
    ) -> Result<TemplateIdentity> {
        let (vm_net, uds_path) = match attach_network_and_vsock(template_id, spec) {
            Ok(v) => v,
            Err(err) => {
                let _ = network().detach(template_id);
                let _ = vsock_manager().detach(template_id);
                return Err(err);
            }
        };

        let mut rootfs_cloned = false;
        if disk_gb > 0 {
            let stack_name = stack.ok_or_else(|| {
                SandboxError::Validation(
                    "SANDBOX_FC_WARM_DISK_GB > 0 requires SANDBOX_FIRECRACKER_DEFAULT_STACK \
                     so the warm template has a rootfs template to clone"
                        .to_string(),
                )
            })?;
            let target_bytes = disk_gb.saturating_mul(1024 * 1024 * 1024);
            match rootfs_registry().clone_for_vm_with_size(template_id, stack_name, target_bytes) {
                Ok(rootfs) => {
                    spec.rootfs = Some(rootfs.path);
                    rootfs_cloned = true;
                }
                Err(err) => {
                    let _ = network().detach(template_id);
                    let _ = vsock_manager().detach(template_id);
                    return Err(map_vm_error("warm rootfs_clone", template_id, err));
                }
            }
        }

        Ok(TemplateIdentity {
            guest_ip: Some(vm_net.guest_ip),
            metadata_uds: Some(uds_path),
            rootfs_cloned,
        })
    }

    async fn await_guest_ready(
        &self,
        template_id: &str,
        identity: &TemplateIdentity,
    ) -> Result<()> {
        let uds_path = identity.metadata_uds.clone().ok_or_else(|| {
            SandboxError::Unavailable(format!(
                "warm template {template_id} composed without a vsock UDS"
            ))
        })?;
        let vm_id = template_id.to_string();
        tokio::task::spawn_blocking(move || -> std::result::Result<(), VmRuntimeError> {
            let client = GuestMetadataClient::new(uds_path, GuestMetadataConfig::from_env());
            let mut conn = client.connect()?;
            conn.ping()
        })
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("warm guest-ready join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("warm guest_ready", template_id, e))
    }

    async fn prepare_snapshot_source(&self, template_id: &str, identity: &TemplateIdentity) {
        // The vmstate records the template's vsock UDS path; the restored
        // entry's Firecracker binds a fresh listener there. The paused
        // template still holds the old listener fd — unlinking the file
        // orphans it (the template never serves again; it exists only as
        // the snapshot's durable home) and frees the path for the entry.
        if let Some(uds) = &identity.metadata_uds
            && let Err(err) = std::fs::remove_file(uds)
        {
            tracing::warn!(
                template_id,
                uds = %uds.display(),
                %err,
                "failed to unlink warm template vsock UDS; entry restore may fail to bind"
            );
        }
    }

    async fn attach_rider(&self, rider_id: &str) -> Result<Option<NetworkInterface>> {
        // The rider TAP carries the claimed VM's traffic; the guest keeps
        // the MAC + IP recorded in the snapshot (Firecracker's restore
        // override swaps the host device only), so no guest_mac is set.
        let vm_net = network()
            .attach(rider_id)
            .map_err(|e| map_vm_error("warm rider network_attach", rider_id, e))?;
        Ok(Some(NetworkInterface {
            iface_id: "eth0".into(),
            host_dev_name: vm_net.tap_name,
            guest_mac: None,
            rx_rate_limiter: None,
            tx_rate_limiter: None,
        }))
    }

    async fn release_template(&self, template_id: &str, identity: &TemplateIdentity, all: bool) {
        if let Err(err) = network().detach(template_id) {
            tracing::warn!(template_id, ?err, "failed to detach warm template network");
        }
        if all {
            if let Err(err) = vsock_manager().detach(template_id) {
                tracing::warn!(template_id, ?err, "failed to detach warm template vsock");
            }
            if identity.rootfs_cloned
                && let Err(err) = rootfs_registry().release(template_id)
            {
                tracing::warn!(
                    template_id,
                    ?err,
                    "failed to release warm template rootfs clone"
                );
            }
        }
    }
}

/// Process-wide warm-serving engine. `None` until the first create with
/// `SANDBOX_FC_WARM_POOL_SIZE > 0` — constructing it spins up the pool's
/// refill thread, which a disabled deployment must never pay for.
static WARM_SERVING: OnceLock<Arc<WarmServing<FirecrackerVmProvider>>> = OnceLock::new();

/// Whether the warm engine was ever constructed. Test-visible so the
/// admission-order invariant ("a rejected create never touches the pool")
/// can be pinned from integration tests.
#[cfg(any(test, feature = "test-utils"))]
pub fn warm_pool_initialized_for_tests() -> bool {
    WARM_SERVING.get().is_some()
}

/// Drive [`reconcile_warm_orphans`] from an integration test.
#[cfg(any(test, feature = "test-utils"))]
pub fn reconcile_warm_orphans_for_tests() {
    reconcile_warm_orphans();
}

/// Try to serve the create from the warm pool. Returns the typed outcome;
/// the caller logs misses and proceeds with the cold path.
///
/// MUST only be called from [`create_and_start`]: the runtime layer runs
/// `admit_sandbox_resources` + `enforce_sandbox_count_limit` (under the
/// creation permit) before dispatching here, which is what makes a warm
/// claim count against `SANDBOX_MAX_COUNT` / the host memory budget exactly
/// like a cold boot. Pool inventory itself is not a sandbox and is never
/// admission-accounted — see the invariant note in [`firecracker_warm`].
async fn warm_claim(req: &FirecrackerCreateRequest) -> Result<WarmOutcome> {
    let pool_size = firecracker_warm::configured_pool_size()?;
    if pool_size == 0 {
        return Ok(WarmOutcome::Miss(firecracker_warm::WarmMiss::Disabled));
    }

    let serving = match WARM_SERVING.get() {
        Some(s) => s,
        None => {
            let entry_max_age = firecracker_warm::configured_entry_max_age()?;
            let disk_gb = firecracker_warm::configured_warm_disk_gb()?;
            // Workspace defaults define the pooled machine shape; requests
            // must match them (or leave cpu/mem unset) to claim.
            let fc_config = FirecrackerConfig::from_env();
            let settings = WarmSettings {
                pool_size,
                stack: default_stack_name(),
                disk_gb,
                vcpu_count: fc_config.vcpu_count,
                mem_size_mib: fc_config.mem_size_mib,
                entry_max_age,
            };
            WARM_SERVING.get_or_init(|| {
                // Reap warm VMs orphaned by a previous operator process BEFORE
                // seeding: a fresh process resets the generation counter to 0
                // and re-mints the same ids, so reaping after the first seed
                // would kill the just-seeded generation.
                reconcile_warm_orphans();
                Arc::new(WarmServing::new(
                    provider().clone(),
                    Arc::new(OperatorWarmHost),
                    settings,
                ))
            })
        }
    };

    serving.ensure_seeding();
    Ok(serving
        .claim(&WarmClaimRequest {
            sandbox_id: req.session_id.clone(),
            image: req.image.clone(),
            cpu_cores: req.cpu_cores,
            memory_mb: req.memory_mb,
            disk_gb: req.disk_gb,
        })
        .await)
}

/// Finish provisioning a warm-claimed VM: DNAT for requested ports, per-VM
/// env + sidecar token over the inherited vsock, attachment bookkeeping,
/// endpoint from the inherited guest IP. Any failure destroys the claimed
/// VM and propagates — the claim already consumed the generation, and the
/// same environmental cause would fail a cold boot's identical steps.
async fn finish_warm_claim(
    req: FirecrackerCreateRequest,
    claim: WarmClaim,
) -> Result<FirecrackerProvisionResult> {
    let vm_id = req.session_id.clone();
    let guest_ip: Ipv4Addr = claim.guest_ip.ok_or_else(|| {
        SandboxError::Unavailable(format!(
            "warm claim for {vm_id} carried no guest IP; the operator warm host always \
             composes one — this indicates a non-production WarmHost in a production path"
        ))
    })?;
    let metadata_uds = claim.metadata_uds.clone().ok_or_else(|| {
        SandboxError::Unavailable(format!(
            "warm claim for {vm_id} carried no metadata UDS; env injection is impossible"
        ))
    })?;

    async fn teardown_warm_claim(vm_id: &str, dnat_rule_count: usize, lineage: WarmLineage) {
        let cleanup_id = vm_id.to_string();
        let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
        release_attachments(
            vm_id,
            &VmAttachments {
                network_attached: false,
                vsock_attached: false,
                dnat_rule_count,
                rootfs_cloned: false,
                warm: Some(lineage),
            },
        );
    }

    let mut dnat_rule_count = 0usize;
    for port in &req.ports {
        if let Err(err) = firecracker_dnat::install_port_forward(&vm_id, guest_ip, *port) {
            tracing::error!(
                vm_id = %vm_id,
                host_port = port.host_port,
                container_port = port.container_port,
                %err,
                "failed to install DNAT for warm-claimed port forward; tearing down"
            );
            teardown_warm_claim(&vm_id, dnat_rule_count, claim.lineage.clone()).await;
            return Err(SandboxError::Unavailable(format!(
                "firecracker port forward install failed for warm-claimed {vm_id}: {err}"
            )));
        }
        dnat_rule_count += 1;
    }

    let sidecar_auth_token =
        match inject_runtime_metadata(&vm_id, metadata_uds, req.env.clone()).await {
            Ok(t) => t,
            Err(err) => {
                teardown_warm_claim(&vm_id, dnat_rule_count, claim.lineage.clone()).await;
                return Err(err);
            }
        };

    // Persist the lineage durably so a delete or reconcile after an operator
    // restart (which loses the in-memory attachment map) can still release the
    // template's rootfs clone + vsock CID and the rider TAP.
    crate::firecracker_lineage::record(&vm_id, &claim.lineage);

    record_attachments(
        &vm_id,
        VmAttachments {
            network_attached: false,
            vsock_attached: false,
            dnat_rule_count,
            rootfs_cloned: false,
            warm: Some(claim.lineage),
        },
    );

    let endpoint = format!("http://{guest_ip}:{}", sidecar_port());
    Ok(FirecrackerProvisionResult {
        container: FirecrackerContainer {
            id: vm_id,
            endpoint: Some(endpoint),
        },
        sidecar_auth_token: Some(sidecar_auth_token),
    })
}

/// Guest IP a warm-claimed sandbox inherited from its template, if any.
/// Consulted by [`start`] so resume rebuilds the endpoint from the IP the
/// guest actually has instead of the sandbox-id-derived allocation.
fn warm_guest_ip(vm_id: &str) -> Option<Ipv4Addr> {
    attachments_map()
        .lock()
        .ok()?
        .get(vm_id)?
        .warm
        .as_ref()?
        .guest_ip
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
/// vsock, rootfs (when sized), and any requested host port forwards; then
/// inject per-VM env and the sidecar auth token into the guest via the
/// metadata service before returning.
///
/// On any failure after VM creation, the VM is destroyed and all attached
/// host resources are released before returning. On success, a record of
/// the attachments is stored so a later `delete` releases them.
pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    // Warm-pool serving first (SANDBOX_FC_WARM_POOL_SIZE > 0): a claim
    // hands off a pre-restored VM via rename + resume. Any miss is logged
    // with its typed reason and falls through to the cold boot below — the
    // one designed fallback in this module. Invalid warm configuration is a
    // hard error, never a silent cold boot.
    match warm_claim(&req).await? {
        WarmOutcome::Claimed(claim) => return finish_warm_claim(req, claim).await,
        WarmOutcome::Miss(firecracker_warm::WarmMiss::Disabled) => {}
        WarmOutcome::Miss(miss) => {
            tracing::info!(
                sandbox_id = %req.session_id,
                reason = %miss,
                "firecracker warm-pool miss; falling back to cold boot"
            );
        }
    }

    let vm_id = req.session_id.clone();
    let mut spec = spec_from_request(&req);

    // Compose the per-VM TAP + vsock binding pre-spawn. The returned
    // `VmNetwork` carries the guest IP that the sidecar endpoint URL is
    // built from; the UDS path is used post-boot to push env + secrets.
    let (vm_net, vsock_uds_path) = match attach_network_and_vsock(&vm_id, &mut spec) {
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

    // Per-VM disk sizing via the rootfs registry. Skipped entirely when
    // `disk_gb == 0` so the provider's default rootfs is reused untouched.
    let rootfs_cloned = match attach_rootfs(&vm_id, &req, &mut spec) {
        Ok(v) => v,
        Err(err) => {
            release_attachments(&vm_id, &VmAttachments::cold(0, false));
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
        release_attachments(&vm_id, &VmAttachments::cold(0, rootfs_cloned));
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
        release_attachments(&vm_id, &VmAttachments::cold(0, rootfs_cloned));
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
            release_attachments(&vm_id, &VmAttachments::cold(dnat_rule_count, rootfs_cloned));
            return Err(SandboxError::Unavailable(format!(
                "firecracker port forward install failed for {vm_id}: {err}"
            )));
        }
        dnat_rule_count += 1;
    }

    // Inject per-VM env + sidecar auth token into the guest. Any failure
    // here is a real runtime error (the guest daemon is unreachable, the
    // env is malformed, etc.) — propagate it after rolling back the VM so
    // the caller does not end up with an inaccessible sandbox.
    let sidecar_auth_token = match inject_runtime_metadata(&vm_id, vsock_uds_path, req.env).await {
        Ok(t) => t,
        Err(err) => {
            let cleanup_id = vm_id.clone();
            let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
            release_attachments(&vm_id, &VmAttachments::cold(dnat_rule_count, rootfs_cloned));
            return Err(err);
        }
    };

    record_attachments(&vm_id, VmAttachments::cold(dnat_rule_count, rootfs_cloned));

    let endpoint = format!("http://{}:{}", vm_net.guest_ip, sidecar_port());

    Ok(FirecrackerProvisionResult {
        container: FirecrackerContainer {
            id: vm_id,
            endpoint: Some(endpoint),
        },
        sidecar_auth_token: Some(sidecar_auth_token),
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

    // Warm-claimed VMs keep the guest IP baked into their template's
    // snapshot; deriving one from the sandbox id here would report an
    // endpoint the guest never configured.
    let guest_ip = match warm_guest_ip(container_id) {
        Some(ip) => ip,
        None => {
            // `NetworkManager::attach` is idempotent: for a known vm_id it
            // returns the same TAP / IP allocation. Resuming a VM whose TAP
            // was torn down out-of-band recreates it; this matches what an
            // operator would do by hand for debugging.
            network()
                .attach(container_id)
                .map_err(|e| map_vm_error("network_attach_resume", container_id, e))?
                .guest_ip
        }
    };
    let endpoint = format!("http://{}:{}", guest_ip, sidecar_port());

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
/// (DNAT rules, vsock CID, TAP, rootfs clone).
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
    // Clear the durable lineage entry regardless; it is released either through
    // the in-memory attachments (normal delete) or reconstructed below.
    let persisted_lineage = crate::firecracker_lineage::take(&vm_id);
    if let Some(attachments) = take_attachments(&vm_id) {
        release_attachments(&vm_id, &attachments);
    } else {
        // The operator restarted and lost the in-memory attachment map. Release
        // the sandbox's own-id resources (best-effort, all idempotent) plus the
        // durably-persisted warm lineage — the template's rootfs clone + vsock
        // CID and the rider TAP, which live under ids other than this sandbox's
        // and would otherwise leak.
        release_attachments(
            &vm_id,
            &VmAttachments {
                network_attached: true,
                vsock_attached: true,
                dnat_rule_count: 1,
                rootfs_cloned: true,
                warm: persisted_lineage,
            },
        );
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
    fn reclaim_clone_predicate_is_data_loss_safe() {
        use std::collections::HashSet;
        let referenced: HashSet<String> = ["fcwarm-g0-tpl".to_string()].into_iter().collect();
        // A live sandbox's own-id (non-warm) clone is NEVER reclaimed — the
        // data-loss guard: removing it would destroy a running workspace.
        assert!(!should_reclaim_clone("sandbox-live-1a2b3c", &referenced));
        assert!(!should_reclaim_clone("a1b2c3d4-uuid", &referenced));
        // A warm template a live sandbox's lineage still references is kept.
        assert!(!should_reclaim_clone("fcwarm-g0-tpl", &referenced));
        // A warm-template clone no live lineage references is reclaimed.
        assert!(should_reclaim_clone("fcwarm-g1-tpl", &referenced));
        assert!(should_reclaim_clone(
            "warm-node20-1_0_0-seed",
            &HashSet::new()
        ));
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
    fn map_vm_error_translates_guest_metadata_to_unavailable() {
        // Regression: when the guest daemon is unreachable or rejects a
        // request, we surface it as `Unavailable` (operator can install the
        // daemon, restart the VM, etc.) — never as `Unsupported`, which
        // would falsely claim the feature is unimplemented.
        let err = map_vm_error(
            "test",
            "vm-1",
            VmRuntimeError::GuestMetadata("daemon did not reply".into()),
        );
        assert!(matches!(err, SandboxError::Unavailable(_)), "got {err:?}");
        assert!(err.to_string().contains("daemon did not reply"));
    }

    #[test]
    fn mint_sidecar_token_is_64_hex_chars_and_unique() {
        let a = mint_sidecar_token();
        let b = mint_sidecar_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn attachments_roundtrip_is_take_once() {
        // Regression: a second `delete` after the first must not see the
        // attachments again (otherwise we'd double-release DNAT rules,
        // which is fine, but also issue a misleading WARN log).
        let vm_id = "vm-attach-roundtrip";
        // Ensure no leftover state from a previous test run.
        let _ = take_attachments(vm_id);
        record_attachments(vm_id, VmAttachments::cold(2, true));
        let first = take_attachments(vm_id).expect("first take returns recorded value");
        assert_eq!(first.dnat_rule_count, 2);
        assert!(first.rootfs_cloned);
        assert!(take_attachments(vm_id).is_none());
    }

    #[test]
    fn default_stack_name_round_trips_through_env() {
        // Use a hermetic guard: capture the prior value, set, observe, restore.
        // SAFETY: the surrounding tests acquire a process-wide env mutex in
        // `lib.rs` (TEST_ENV_GUARD); this unit test is the only place in
        // this module that mutates env, so a localised lock would buy
        // nothing — the helper just round-trips and restores.
        let prior = std::env::var("SANDBOX_FIRECRACKER_DEFAULT_STACK").ok();
        unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", "node-20") };
        assert_eq!(default_stack_name().as_deref(), Some("node-20"));
        unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", "   ") };
        assert!(
            default_stack_name().is_none(),
            "empty/whitespace must be None"
        );
        match prior {
            Some(v) => unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", v) },
            None => unsafe { std::env::remove_var("SANDBOX_FIRECRACKER_DEFAULT_STACK") },
        }
    }
}
