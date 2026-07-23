//! Warm-pool configuration knobs + host-memory reservation.
//!
//! Every knob mirrors an `SANDBOX_FC_WARM_*` sibling in
//! [`crate::firecracker_warm::config`]; the parsing discipline is identical
//! (absent/empty/`0` disables, unparseable is a hard [`SandboxError::Validation`],
//! never a silent disable).

use super::*;

/// Parse `SANDBOX_DOCKER_WARM_POOL_SIZE`. Absent/empty/`0` disables warm
/// serving; anything unparseable is a hard configuration error — a typo must
/// never silently disable the pool. Mirror of
/// [`crate::firecracker_warm::configured_pool_size`].
pub(crate) fn configured_pool_size() -> Result<usize> {
    match std::env::var("SANDBOX_DOCKER_WARM_POOL_SIZE") {
        Err(_) => Ok(0),
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            trimmed.parse::<usize>().map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_DOCKER_WARM_POOL_SIZE must be a non-negative integer, got {trimmed:?}"
                ))
            })
        }
    }
}

/// The pooled per-container memory (MB).
///
/// Unlike Firecracker — whose warm pool reads `mem_size_mib` from
/// `FirecrackerConfig::from_env()` — the Docker backend has no per-container
/// memory default in [`SidecarRuntimeConfig`] (`SANDBOX_MAX_MEMORY_MB` defaults
/// to `0` = unlimited). So when the pool is enabled this value is **required**:
/// the host-memory-budget reservation ([`reserved_host_memory_mb`]) cannot be
/// computed without it, and a warm container created with no memory cap would
/// let the pool over-commit host RAM invisibly. Requiring it is the fail-closed
/// choice — never a silent `0`.
///
/// Returns `0` only when the pool is disabled (`pool_size == 0`), so a
/// Firecracker-only or plain-Docker host that never sets the pool size pays
/// nothing and sees no error.
pub(crate) fn configured_warm_memory_mb(pool_size: usize) -> Result<u64> {
    if pool_size == 0 {
        return Ok(0);
    }
    match std::env::var("SANDBOX_DOCKER_WARM_MEMORY_MB") {
        Ok(raw) if !raw.trim().is_empty() => {
            let mb = raw.trim().parse::<u64>().map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_DOCKER_WARM_MEMORY_MB must be a non-negative integer, got {:?}",
                    raw.trim()
                ))
            })?;
            if mb == 0 {
                return Err(SandboxError::Validation(
                    "SANDBOX_DOCKER_WARM_MEMORY_MB must be > 0 when SANDBOX_DOCKER_WARM_POOL_SIZE \
                     is set: the host memory budget reserves pool_size × memory_mb, and a warm \
                     container with no memory cap would over-commit host RAM."
                        .to_string(),
                ));
            }
            Ok(mb)
        }
        _ => Err(SandboxError::Validation(
            "SANDBOX_DOCKER_WARM_MEMORY_MB is required when SANDBOX_DOCKER_WARM_POOL_SIZE is set: \
             the Docker backend has no default per-container memory, and the host memory budget \
             must be able to account for the warm pool's standing footprint."
                .to_string(),
        )),
    }
}

/// The pooled per-container CPU cores. `0` (default) leaves the warm container
/// uncapped, matching a request that also leaves `cpu_cores` unset (0). No
/// budget reservation is made for CPU — idle warm containers time-share CPU and
/// pin only RAM, the same rationale the Firecracker pool uses.
pub(crate) fn configured_warm_cpu_cores() -> Result<u64> {
    match std::env::var("SANDBOX_DOCKER_WARM_CPU_CORES") {
        Err(_) => Ok(0),
        Ok(raw) if raw.trim().is_empty() => Ok(0),
        Ok(raw) => raw.trim().parse::<u64>().map_err(|_| {
            SandboxError::Validation(format!(
                "SANDBOX_DOCKER_WARM_CPU_CORES must be a non-negative integer, got {:?}",
                raw.trim()
            ))
        }),
    }
}

/// The pooled sidecar image. Defaults to the operator's `SIDECAR_IMAGE`
/// ([`SidecarRuntimeConfig::image`]). A request must name this image (or leave
/// `image` empty and default to it) to claim from the pool.
pub(crate) fn configured_warm_image() -> String {
    match std::env::var("SANDBOX_DOCKER_WARM_IMAGE") {
        Ok(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
        _ => SidecarRuntimeConfig::load().image.clone(),
    }
}

/// The base env JSON baked into every warm container. A request qualifies for a
/// warm claim only if its `env_json` matches this AND it carries no user env
/// (see the shape gate). Defaults to empty (`{}`) — the "no custom base env"
/// default shape. Operators whose common request carries a fixed base env set
/// this so warm can serve it.
pub(crate) fn configured_warm_base_env_json() -> String {
    std::env::var("SANDBOX_DOCKER_WARM_BASE_ENV_JSON")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_default()
}

/// The capabilities JSON baked into every warm container (drives
/// `SIDECAR_CAPABILITIES`, which is create-time immutable). Defaults to empty
/// (no extra capabilities). A request qualifies only if its capabilities parse
/// to the same set.
pub(crate) fn configured_warm_capabilities_json() -> String {
    std::env::var("SANDBOX_DOCKER_WARM_CAPABILITIES_JSON")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_default()
}

/// Parse `SANDBOX_DOCKER_WARM_MAX_AGE_SECS` (pool-entry age eviction). Default
/// 3600s. An over-age warm container is reaped and refilled so pooled entries
/// never drift arbitrarily stale. Mirror of
/// `SANDBOX_FC_WARM_MAX_AGE_SECS`.
pub(crate) fn configured_entry_max_age() -> Result<Duration> {
    match std::env::var("SANDBOX_DOCKER_WARM_MAX_AGE_SECS") {
        Err(_) => Ok(Duration::from_secs(3600)),
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_DOCKER_WARM_MAX_AGE_SECS must be a non-negative integer, got {raw:?}"
                ))
            }),
    }
}

/// Standing host-memory footprint the warm pool reserves against
/// `SANDBOX_HOST_MEMORY_BUDGET_MB`, so admission accounts for pool inventory
/// that never enters the sandbox store. Zero when warm serving is disabled
/// (`SANDBOX_DOCKER_WARM_POOL_SIZE` unset/0), so Firecracker-only hosts are
/// unaffected.
///
/// Factor 1 (not Firecracker's 2): a Docker warm entry is a single container,
/// with no separate paused-template + pre-restored-entry pair.
///
/// Deterministic from config — a fixed reservation, not a live pool query: the
/// engine is lazily built only on the first Docker create (after admission), so
/// a live query would report 0 reserved on the very create that seeds the pool
/// and let it over-commit (identical reasoning to
/// [`crate::firecracker_warm::reserved_host_memory_mb`]).
pub(crate) fn reserved_host_memory_mb() -> Result<u64> {
    let pool_size = configured_pool_size()?;
    if pool_size == 0 {
        return Ok(0);
    }
    let memory_mb = configured_warm_memory_mb(pool_size)?;
    Ok((pool_size as u64).saturating_mul(memory_mb))
}

/// Load the full warm-pool settings, or `None` when the pool is disabled.
///
/// Called on every Docker create (cheap env reads). A misconfiguration
/// (unparseable pool size, or pool enabled without a memory value) is a hard
/// error that fails the create loudly rather than silently degrading — the same
/// fail-loud contract as the Firecracker pool.
pub(crate) fn load_settings() -> Result<Option<DockerWarmSettings>> {
    let pool_size = configured_pool_size()?;
    if pool_size == 0 {
        return Ok(None);
    }
    Ok(Some(DockerWarmSettings {
        pool_size,
        image: configured_warm_image(),
        cpu_cores: configured_warm_cpu_cores()?,
        memory_mb: configured_warm_memory_mb(pool_size)?,
        base_env_json: configured_warm_base_env_json(),
        capabilities_json: configured_warm_capabilities_json(),
        entry_max_age: configured_entry_max_age()?,
    }))
}

/// The shape a warm container is seeded at and a create request must match (or
/// leave unset) to be servable from the pool.
#[derive(Debug, Clone)]
pub(crate) struct DockerWarmSettings {
    /// Number of warm containers to keep claimable.
    pub pool_size: usize,
    /// Sidecar image baked into every warm container.
    pub image: String,
    /// CPU cores the warm container is capped at (`0` = uncapped).
    pub cpu_cores: u64,
    /// Memory (MB) the warm container is capped at (always `> 0` when enabled).
    pub memory_mb: u64,
    /// Base env JSON baked into the warm container (immutable after create).
    pub base_env_json: String,
    /// Capabilities JSON baked into the warm container (immutable after create).
    pub capabilities_json: String,
    /// Pool-entry age eviction window.
    pub entry_max_age: Duration,
}
