//! Warm-pool configuration knobs + host-memory reservation.

use super::*;

/// Snapshot name every generation's golden image is stored under.
pub(crate) const GOLDEN_SNAPSHOT_ID: &str = "golden";

/// Parse `SANDBOX_FC_WARM_POOL_SIZE`. Absent or `0` disables warm serving;
/// anything unparseable is a hard configuration error — a typo must never
/// silently disable the pool.
pub(crate) fn configured_pool_size() -> Result<usize> {
    match std::env::var("SANDBOX_FC_WARM_POOL_SIZE") {
        Err(_) => Ok(0),
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            trimmed.parse::<usize>().map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_FC_WARM_POOL_SIZE must be a non-negative integer, got {trimmed:?}"
                ))
            })
        }
    }
}

/// Paused template + pre-restored entry per generation — the `file`-backend
/// worst case, kept for `uffd` too so the budget guard stays fail-closed.
const WARM_GENERATION_MEM_FACTOR: u64 = 2;

/// Standing host-memory footprint the warm pool reserves against
/// `SANDBOX_HOST_MEMORY_BUDGET_MB`, so admission accounts for pool inventory
/// that never enters the sandbox store. Zero when warm serving is disabled
/// (`SANDBOX_FC_WARM_POOL_SIZE` unset/0), so Docker-only hosts are unaffected.
///
/// Deterministic from config — a fixed reservation, not a live pool query: the
/// engine is lazily built only on the first Firecracker create (after
/// admission), so a live query would report 0 reserved on the very create that
/// is about to seed the pool and let it over-commit. `mem_size_mib` is read
/// from the same `FirecrackerConfig::from_env()` the engine bakes into every
/// generation's golden snapshot, so the reservation matches the real footprint.
pub(crate) fn reserved_host_memory_mb() -> Result<u64> {
    let pool_size = configured_pool_size()? as u64;
    if pool_size == 0 {
        return Ok(0);
    }
    let mem_size_mib =
        microvm_runtime::adapters::firecracker::FirecrackerConfig::from_env().mem_size_mib as u64;
    Ok(pool_size
        .saturating_mul(WARM_GENERATION_MEM_FACTOR)
        .saturating_mul(mem_size_mib))
}

/// Parse `SANDBOX_FC_WARM_MAX_AGE_SECS` (pool-entry age eviction). Default
/// 3600s: evictions force a snapshot re-restore, so a long default keeps the
/// pool quiet; operators lower it if they want fresher entries.
pub(crate) fn configured_entry_max_age() -> Result<Duration> {
    match std::env::var("SANDBOX_FC_WARM_MAX_AGE_SECS") {
        Err(_) => Ok(Duration::from_secs(3600)),
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_FC_WARM_MAX_AGE_SECS must be a non-negative integer, got {raw:?}"
                ))
            }),
    }
}

/// Parse `SANDBOX_FC_WARM_DISK_GB` — the per-generation rootfs clone size.
/// `0` (default) keeps the provider's workspace-default rootfs (no clone),
/// matching the cold path's `disk_gb == 0` semantics.
pub(crate) fn configured_warm_disk_gb() -> Result<u64> {
    match std::env::var("SANDBOX_FC_WARM_DISK_GB") {
        Err(_) => Ok(0),
        Ok(raw) => raw.trim().parse::<u64>().map_err(|_| {
            SandboxError::Validation(format!(
                "SANDBOX_FC_WARM_DISK_GB must be a non-negative integer, got {raw:?}"
            ))
        }),
    }
}

/// Shape a warm generation is provisioned at and a create request must match
/// (or leave unset) to be servable from the pool.
#[derive(Debug, Clone)]
pub(crate) struct WarmSettings {
    /// Number of generations to keep claimable.
    pub pool_size: usize,
    /// Stack the template's rootfs is cloned from when `disk_gb > 0`
    /// (`SANDBOX_FIRECRACKER_DEFAULT_STACK`). Irrelevant when `disk_gb == 0`
    /// — the cold path ignores `image` in that case too, so warm/cold serve
    /// identical guests.
    pub stack: Option<String>,
    /// Rootfs clone size for the template (`SANDBOX_FC_WARM_DISK_GB`).
    pub disk_gb: u64,
    /// vCPU count baked into the golden snapshot (provider workspace default).
    pub vcpu_count: u8,
    /// Memory baked into the golden snapshot (provider workspace default).
    pub mem_size_mib: u32,
    /// Pool-entry age eviction (`SANDBOX_FC_WARM_MAX_AGE_SECS`).
    pub entry_max_age: Duration,
}
