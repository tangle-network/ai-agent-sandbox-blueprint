//! Process-wide provider / network / vsock / rootfs singletons + per-VM attachments.

use super::*;

/// Single process-wide handle to the Firecracker provider.
///
/// The provider is `Clone`able but keeps an internal `Arc<RwLock<…>>` of VM
/// records, so we only want one instance per operator process — multiple
/// instances would diverge state and leak Firecracker child processes.
pub(crate) fn provider() -> &'static FirecrackerVmProvider {
    static PROVIDER: OnceLock<FirecrackerVmProvider> = OnceLock::new();
    PROVIDER.get_or_init(FirecrackerVmProvider::from_env)
}

/// Process-wide [`NetworkManager`]. The manager is stateless (the kernel is
/// the source of truth for bridge / TAP / iptables), so sharing one instance
/// across the process is safe and avoids re-reading env vars per call.
pub(crate) fn network() -> &'static NetworkManager {
    static NETWORK: OnceLock<NetworkManager> = OnceLock::new();
    NETWORK.get_or_init(NetworkManager::from_env)
}

/// Process-wide [`VsockManager`]. Unlike [`network`], the vsock manager owns
/// an in-process CID allocation map, so all `attach`/`detach` calls must go
/// through the same instance to avoid CID double-allocation.
pub(crate) fn vsock_manager() -> &'static VsockManager {
    static VSOCK: OnceLock<VsockManager> = OnceLock::new();
    VSOCK.get_or_init(VsockManager::from_env)
}

/// Process-wide [`RootfsRegistry`]. The registry only caches `(path, mtime)
/// → sha256`, which is safe to share; the per-VM clone slots it produces are
/// keyed by `vm_id` so callers cannot collide across sandboxes.
pub(crate) fn rootfs_registry() -> &'static RootfsRegistry {
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
pub(crate) struct VmAttachments {
    pub(crate) network_attached: bool,
    pub(crate) vsock_attached: bool,
    /// Number of installed DNAT rules. Used as a tombstone — non-zero means
    /// we created at least one rule and the per-VM chain must be torn down.
    pub(crate) dnat_rule_count: usize,
    /// `true` iff a per-VM rootfs clone was created and must be released
    /// on delete. `false` for VMs that reused the provider's default rootfs.
    pub(crate) rootfs_cloned: bool,
    /// Warm-claim lineage: host resources this sandbox holds under ids other
    /// than its own (rider TAP, template vsock CID, template rootfs clone).
    /// `None` for cold-booted VMs. This in-memory copy serves the normal
    /// delete; a durable copy is persisted by [`crate::firecracker_lineage`] so
    /// a delete or reconcile after an operator restart still releases the
    /// aliases instead of leaking them.
    pub(crate) warm: Option<WarmLineage>,
}

impl VmAttachments {
    pub(crate) fn cold(dnat_rule_count: usize, rootfs_cloned: bool) -> Self {
        Self {
            network_attached: true,
            vsock_attached: true,
            dnat_rule_count,
            rootfs_cloned,
            warm: None,
        }
    }
}

pub(crate) fn attachments_map() -> &'static Mutex<HashMap<String, Arc<VmAttachments>>> {
    static MAP: OnceLock<Mutex<HashMap<String, Arc<VmAttachments>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn record_attachments(vm_id: &str, value: VmAttachments) {
    if let Ok(mut guard) = attachments_map().lock() {
        guard.insert(vm_id.to_string(), Arc::new(value));
    }
}

pub(crate) fn take_attachments(vm_id: &str) -> Option<Arc<VmAttachments>> {
    attachments_map().lock().ok()?.remove(vm_id)
}

/// Sidecar HTTP port the guest listens on. The same value is also baked
/// into the guest's `SIDECAR_PORT` env var by the runtime layer; reading
/// it here keeps the URL construction colocated with the FC wiring.
pub(crate) fn sidecar_port() -> u16 {
    std::env::var("SIDECAR_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080)
}

/// Stack name used when `req.image` is empty. `None` means "fall back to
/// the workspace default rootfs path the provider was constructed with" —
/// no per-VM clone is performed in that case.
pub(crate) fn default_stack_name() -> Option<String> {
    std::env::var("SANDBOX_FIRECRACKER_DEFAULT_STACK")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
