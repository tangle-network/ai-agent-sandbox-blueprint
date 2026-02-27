//! Circuit breaker for unhealthy sidecars.
//!
//! Tracks per-sandbox health state. When a sidecar call fails with a connection
//! error or timeout, [`mark_unhealthy`] records the sandbox as down. Subsequent
//! calls to [`check_health`] during the cooldown window return an immediate
//! error, avoiding pointless retries against a known-broken sidecar.
//!
//! The cooldown period defaults to 30 seconds and can be overridden via the
//! `CIRCUIT_BREAKER_COOLDOWN_SECS` environment variable.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use once_cell::sync::Lazy;

use crate::error::{Result, SandboxError};

/// Default cooldown before a sandbox marked unhealthy can be retried.
const DEFAULT_COOLDOWN_SECS: u64 = 30;

/// Interval between GC sweeps — entries older than 2x cooldown are removed.
const GC_INTERVAL_SECS: u64 = 120;

/// Map of sandbox ID -> instant when it was marked unhealthy.
static UNHEALTHY: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Tracks the last time GC ran to avoid scanning on every call.
static LAST_GC: Lazy<Mutex<Instant>> = Lazy::new(|| Mutex::new(Instant::now()));

/// Read the configured cooldown in seconds.
fn cooldown_secs() -> u64 {
    std::env::var("CIRCUIT_BREAKER_COOLDOWN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_COOLDOWN_SECS)
}

/// Check whether `sandbox_id` is healthy (i.e., not in cooldown).
///
/// Returns `Ok(())` if the sandbox is healthy or its cooldown has expired.
/// Returns `Err(SandboxError::Unavailable)` if the sandbox was recently marked
/// unhealthy and the cooldown has not yet elapsed.
///
/// Also performs periodic garbage collection of stale entries.
pub fn check_health(sandbox_id: &str) -> Result<()> {
    let cooldown = cooldown_secs();
    let mut map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());

    // Periodic GC: remove entries older than 2x cooldown.
    {
        let mut last_gc = LAST_GC.lock().unwrap_or_else(|e| e.into_inner());
        if last_gc.elapsed().as_secs() >= GC_INTERVAL_SECS {
            let cutoff = Instant::now() - std::time::Duration::from_secs(cooldown * 2);
            map.retain(|_, marked_at| *marked_at > cutoff);
            *last_gc = Instant::now();
        }
    }

    if let Some(marked_at) = map.get(sandbox_id) {
        let elapsed = marked_at.elapsed().as_secs();
        if elapsed < cooldown {
            let remaining = cooldown - elapsed;
            return Err(SandboxError::Unavailable(format!(
                "Sidecar {sandbox_id} is in circuit-breaker cooldown ({remaining}s remaining)"
            )));
        }
        // Cooldown expired — remove the entry and allow the call.
        map.remove(sandbox_id);
    }

    Ok(())
}

/// Mark a sandbox as unhealthy. Subsequent [`check_health`] calls will fail
/// until the cooldown expires.
pub fn mark_unhealthy(sandbox_id: &str) {
    tracing::warn!(sandbox_id, "circuit breaker: marking sidecar unhealthy");
    let mut map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());
    map.insert(sandbox_id.to_string(), Instant::now());
}

/// Mark a sandbox as healthy, clearing any cooldown. Call on successful
/// sidecar interaction.
pub fn mark_healthy(sandbox_id: &str) {
    let mut map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(sandbox_id);
}

/// Alias for [`mark_healthy`] — used when a sandbox is deleted to clean up
/// its circuit-breaker state.
pub fn clear(sandbox_id: &str) {
    mark_healthy(sandbox_id);
}

/// Number of currently tracked unhealthy sandboxes (for testing/metrics).
#[cfg(test)]
fn tracked_count() -> usize {
    UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner()).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Use unique sandbox IDs per test to avoid cross-test interference from the
    // shared static map.
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);
    fn unique_id(prefix: &str) -> String {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{n}")
    }

    #[test]
    fn test_healthy_by_default() {
        let id = unique_id("healthy-default");
        assert!(
            check_health(&id).is_ok(),
            "unknown sandbox should be healthy"
        );
    }

    #[test]
    fn test_mark_unhealthy_blocks() {
        let id = unique_id("unhealthy-blocks");
        mark_unhealthy(&id);
        let err = check_health(&id);
        assert!(err.is_err(), "unhealthy sandbox should be blocked");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("circuit-breaker cooldown"),
            "error message should mention cooldown, got: {msg}"
        );
        // Clean up
        clear(&id);
    }

    #[test]
    fn test_mark_healthy_clears() {
        let id = unique_id("healthy-clears");
        mark_unhealthy(&id);
        assert!(check_health(&id).is_err());
        mark_healthy(&id);
        assert!(
            check_health(&id).is_ok(),
            "sandbox should be healthy after mark_healthy"
        );
    }

    #[test]
    fn test_cooldown_expires() {
        // Insert an entry with an instant far enough in the past that any
        // reasonable cooldown (including the default 30s) would have expired.
        let id = unique_id("cooldown-expires");
        {
            let mut map = UNHEALTHY.lock().unwrap();
            map.insert(
                id.clone(),
                Instant::now() - std::time::Duration::from_secs(60),
            );
        }
        assert!(
            check_health(&id).is_ok(),
            "should be healthy after cooldown expires"
        );
    }

    #[test]
    fn test_gc_removes_stale() {
        // Insert 5 entries with timestamps far in the past so they are stale
        // regardless of the configured cooldown value.
        let ids: Vec<String> = (0..5).map(|_| unique_id("gc-stale")).collect();
        let stale_instant = Instant::now() - std::time::Duration::from_secs(3600);
        {
            let mut map = UNHEALTHY.lock().unwrap();
            for id in &ids {
                map.insert(id.clone(), stale_instant);
            }
        }

        let count_before = tracked_count();
        assert!(count_before >= 5, "should have at least 5 entries");

        // Force GC by setting LAST_GC to the past.
        *LAST_GC.lock().unwrap() =
            Instant::now() - std::time::Duration::from_secs(GC_INTERVAL_SECS + 1);

        // Trigger GC via a check_health call.
        let probe = unique_id("gc-probe");
        let _ = check_health(&probe);

        // The 5 stale entries should have been cleaned up by GC.
        // Verify they are no longer in the map.
        {
            let map = UNHEALTHY.lock().unwrap();
            for id in &ids {
                assert!(!map.contains_key(id), "stale entry {id} should be GC'd");
            }
        }
    }

    #[test]
    fn test_clear_is_alias_for_mark_healthy() {
        let id = unique_id("clear-alias");
        mark_unhealthy(&id);
        assert!(check_health(&id).is_err());
        clear(&id);
        assert!(check_health(&id).is_ok());
    }
}
