//! Warm-serving data types (generation identity, claim, miss/outcome).

use super::*;

/// Identity captured at template compose time, inherited by the claimant.
#[derive(Debug, Clone, Default)]
pub(crate) struct TemplateIdentity {
    /// Guest IP baked into the snapshot (kernel/network state). `None` only
    /// for hosts that compose no network.
    pub guest_ip: Option<Ipv4Addr>,
    /// Host UDS path of the template's vsock — the claimed VM's metadata
    /// listener binds here (recorded in the vmstate).
    pub metadata_uds: Option<PathBuf>,
    /// Whether a per-template rootfs clone was created (must be released
    /// when the claimed sandbox is deleted).
    pub rootfs_cloned: bool,
}

/// One claimable generation.
#[derive(Debug, Clone)]
pub(crate) struct Generation {
    pub template_id: String,
    pub rider_id: String,
    pub stack_key: StackKey,
    pub identity: TemplateIdentity,
    pub rider_attached: bool,
}

/// Everything the create path needs to finish provisioning a claimed VM.
#[derive(Debug, Clone)]
pub(crate) struct WarmClaim {
    /// Guest IP for the endpoint URL + DNAT (template's, via the snapshot).
    pub guest_ip: Option<Ipv4Addr>,
    /// UDS to inject per-sandbox env + sidecar token through.
    pub metadata_uds: Option<PathBuf>,
    /// Alias ids whose host resources now belong to the claimed sandbox.
    pub lineage: WarmLineage,
}

/// Host resources a warm-claimed sandbox holds under ids other than its own.
/// Released at sandbox delete in `firecracker::release_attachments`. Persisted
/// by [`crate::firecracker_lineage`] keyed by sandbox id so a delete or
/// reconcile after an operator restart can still release/reclaim them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct WarmLineage {
    /// Template id: owns the vsock CID allocation and (when
    /// `rootfs_cloned`) the rootfs clone slot backing the claimed VM.
    pub template_id: String,
    /// Rider id: owns the TAP the claimed VM rides. `None` on network-less
    /// hosts.
    pub rider_id: Option<String>,
    pub rootfs_cloned: bool,
    /// Guest IP for endpoint rebuilds on resume (`firecracker::start`).
    pub guest_ip: Option<Ipv4Addr>,
}

/// Typed warm-miss reason. Logged by the create path before the designed
/// fallback to cold boot — the only silent thing about a miss is nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WarmMiss {
    /// `SANDBOX_FC_WARM_POOL_SIZE` is 0/unset.
    Disabled,
    /// Pool enabled but no generation is ready yet (seeding in flight or
    /// all seeds failed so far).
    NotReady,
    /// Ready generations exist but their buckets are empty (entry evicted
    /// and not yet refilled, or refill failing).
    Empty,
    DiskMismatch {
        requested: u64,
        pooled: u64,
    },
    ImageMismatch {
        requested: String,
        pooled: String,
    },
    CpuMismatch {
        requested: u64,
        pooled: u8,
    },
    MemoryMismatch {
        requested: u64,
        pooled: u32,
    },
    /// Handoff rename failed; the entry was destroyed and the generation
    /// returned to the pool for refill.
    RenameFailed(String),
    /// Post-rename resume failed; the claimed VM was destroyed and the
    /// generation returned to the pool for refill.
    ResumeFailed(String),
}

impl std::fmt::Display for WarmMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarmMiss::Disabled => write!(f, "warm pool disabled (SANDBOX_FC_WARM_POOL_SIZE=0)"),
            WarmMiss::NotReady => write!(f, "no warm generation ready (seeding in flight)"),
            WarmMiss::Empty => write!(f, "warm buckets empty (entry evicted, refill pending)"),
            WarmMiss::DiskMismatch { requested, pooled } => write!(
                f,
                "disk_gb mismatch (requested {requested}, pooled {pooled})"
            ),
            WarmMiss::ImageMismatch { requested, pooled } => {
                write!(
                    f,
                    "image mismatch (requested {requested:?}, pooled {pooled:?})"
                )
            }
            WarmMiss::CpuMismatch { requested, pooled } => {
                write!(
                    f,
                    "cpu_cores mismatch (requested {requested}, pooled {pooled})"
                )
            }
            WarmMiss::MemoryMismatch { requested, pooled } => {
                write!(
                    f,
                    "memory_mb mismatch (requested {requested}, pooled {pooled})"
                )
            }
            WarmMiss::RenameFailed(e) => write!(f, "warm handoff rename failed: {e}"),
            WarmMiss::ResumeFailed(e) => write!(f, "warm handoff resume failed: {e}"),
        }
    }
}

/// Outcome of a warm-claim attempt.
#[derive(Debug)]
pub(crate) enum WarmOutcome {
    Claimed(WarmClaim),
    Miss(WarmMiss),
}

/// Request shape the claim gate evaluates (subset of
/// `firecracker::FirecrackerCreateRequest` the engine needs — it must not
/// see env/labels, which are injected after the claim).
#[derive(Debug, Clone)]
pub(crate) struct WarmClaimRequest {
    pub sandbox_id: String,
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
}
