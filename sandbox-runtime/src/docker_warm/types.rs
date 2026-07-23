//! Warm-serving data types (pooled container, claim request/result, typed
//! miss/outcome). Mirrors [`crate::firecracker_warm::types`].

/// One pre-created, pre-started, bootstrapped container sitting in the ready
/// pool. Named `sidecar-warm-<seq>` and labelled `tangle.warm-pool=1` so the
/// startup reconcile can find it; renamed onto the real `sidecar-<sandbox_id>`
/// at claim.
#[derive(Debug, Clone)]
pub(crate) struct WarmContainer {
    /// Docker container id (stable across the claim rename; the rename targets
    /// it by id, not by name).
    pub container_id: String,
    /// Auth token baked into the container's env at seed. It is a random
    /// operator↔sidecar secret (not request-derived), so it is poolable: the
    /// claim copies it verbatim into the store record — no container mutation.
    pub token: String,
    /// Monotonic seed sequence (for logs / label provenance). The pooled image
    /// is not tracked per-entry: it is fixed for the process's lifetime by the
    /// settings, and cross-restart staleness is handled by the reap-always
    /// startup reconcile.
    pub seq: u64,
    /// When the container was seeded (unix seconds), for age eviction.
    pub created_at: u64,
}

/// The request shape the claim gate evaluates — the subset of
/// [`CreateSandboxParams`] the pool needs. Env/labels/timeouts are bound by the
/// caller's record insert, not here.
#[derive(Debug, Clone)]
pub(crate) struct DockerWarmClaimRequest {
    /// Freshly-minted sandbox id the claimed container is renamed onto.
    pub sandbox_id: String,
    /// Effective image (request image, or the operator default when empty).
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    /// Whether SSH was requested (warm seeds with SSH disabled).
    pub ssh_enabled: bool,
    /// Request base env JSON (must match the pooled base env).
    pub env_json: String,
    /// Request user env JSON (must be empty — Docker env is immutable).
    pub user_env_json: String,
    /// Request capabilities JSON (must match the pooled capabilities).
    pub capabilities_json: String,
    /// Number of extra ports requested (must be zero — port bindings are
    /// create-time immutable on Docker).
    pub extra_ports_len: usize,
}

/// Everything the create path needs to finish a warm claim: the reused
/// container id + baked token, plus the endpoint read back from the
/// already-started container.
#[derive(Debug, Clone)]
pub(crate) struct DockerWarmClaim {
    pub container_id: String,
    pub token: String,
    pub sidecar_url: String,
    pub sidecar_port: u16,
    pub ssh_port: Option<u16>,
    pub extra_ports: std::collections::HashMap<u16, u16>,
}

/// Resolved endpoint of a claimed (renamed) container, read back from Docker.
#[derive(Debug, Clone)]
pub(crate) struct ClaimResolved {
    pub sidecar_url: String,
    pub sidecar_port: u16,
    pub ssh_port: Option<u16>,
    pub extra_ports: std::collections::HashMap<u16, u16>,
}

/// A downstream failure of the claim's await stages (after the container was
/// popped from the pool). Every variant maps to a typed [`DockerWarmMiss`]; the
/// container is reaped (it may already be renamed and cannot return to the pool)
/// and the caller cold-falls-back.
#[derive(Debug)]
pub(crate) enum ClaimFailure {
    Rename(String),
    PortResolve(String),
    Unhealthy(String),
}

/// Typed warm-miss reason. Logged by the create path before the designed
/// fallback to cold boot. Mirrors [`crate::firecracker_warm::WarmMiss`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DockerWarmMiss {
    /// `SANDBOX_DOCKER_WARM_POOL_SIZE` is 0/unset.
    Disabled,
    /// Pool enabled but no container is ready yet (seeding in flight).
    NotReady,
    /// Pool enabled, nothing ready and nothing seeding (all seeds failing).
    Empty,
    /// Request names an image other than the pooled one.
    ImageMismatch { requested: String, pooled: String },
    /// Request asks for cpu cores other than the pooled shape.
    CpuMismatch { requested: u64, pooled: u64 },
    /// Request asks for memory other than the pooled shape.
    MemoryMismatch { requested: u64, pooled: u64 },
    /// Request enabled SSH; warm seeds with SSH disabled.
    SshRequested,
    /// Request carries user env; Docker env is create-time immutable.
    UserEnvPresent,
    /// Request's base env differs from the pooled base env.
    BaseEnvMismatch,
    /// Request's capabilities differ from the pooled capabilities.
    CapabilitiesMismatch,
    /// Request asks for extra ports; Docker port bindings are immutable.
    ExtraPortsRequested,
    /// Handoff rename failed; the container was reaped.
    RenameFailed(String),
    /// Post-rename port readback failed; the container was reaped.
    PortResolveFailed(String),
    /// The pooled sidecar was unhealthy at claim; the container was reaped.
    Unhealthy(String),
}

impl DockerWarmMiss {
    pub(crate) fn from_claim_failure(failure: ClaimFailure) -> Self {
        match failure {
            ClaimFailure::Rename(e) => DockerWarmMiss::RenameFailed(e),
            ClaimFailure::PortResolve(e) => DockerWarmMiss::PortResolveFailed(e),
            ClaimFailure::Unhealthy(e) => DockerWarmMiss::Unhealthy(e),
        }
    }
}

impl std::fmt::Display for DockerWarmMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DockerWarmMiss::Disabled => {
                write!(f, "warm pool disabled (SANDBOX_DOCKER_WARM_POOL_SIZE=0)")
            }
            DockerWarmMiss::NotReady => write!(f, "no warm container ready (seeding in flight)"),
            DockerWarmMiss::Empty => {
                write!(f, "warm pool empty (no ready container, none seeding)")
            }
            DockerWarmMiss::ImageMismatch { requested, pooled } => write!(
                f,
                "image mismatch (requested {requested:?}, pooled {pooled:?})"
            ),
            DockerWarmMiss::CpuMismatch { requested, pooled } => write!(
                f,
                "cpu_cores mismatch (requested {requested}, pooled {pooled})"
            ),
            DockerWarmMiss::MemoryMismatch { requested, pooled } => write!(
                f,
                "memory_mb mismatch (requested {requested}, pooled {pooled})"
            ),
            DockerWarmMiss::SshRequested => {
                write!(f, "ssh requested (warm containers seed with ssh disabled)")
            }
            DockerWarmMiss::UserEnvPresent => write!(
                f,
                "user env present (Docker container env is create-time immutable)"
            ),
            DockerWarmMiss::BaseEnvMismatch => {
                write!(f, "base env differs from the pooled warm base env")
            }
            DockerWarmMiss::CapabilitiesMismatch => {
                write!(f, "capabilities differ from the pooled warm capabilities")
            }
            DockerWarmMiss::ExtraPortsRequested => write!(
                f,
                "extra ports requested (Docker port bindings are create-time immutable)"
            ),
            DockerWarmMiss::RenameFailed(e) => write!(f, "warm handoff rename failed: {e}"),
            DockerWarmMiss::PortResolveFailed(e) => write!(f, "warm port readback failed: {e}"),
            DockerWarmMiss::Unhealthy(e) => write!(f, "warm sidecar unhealthy at claim: {e}"),
        }
    }
}

/// Outcome of a warm-claim attempt.
#[derive(Debug)]
pub(crate) enum DockerWarmOutcome {
    Claimed(DockerWarmClaim),
    Miss(DockerWarmMiss),
}
