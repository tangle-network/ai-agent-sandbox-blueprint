//! Three-state circuit breaker for unhealthy sidecars.
//!
//! Tracks per-sandbox health state with three states:
//! - **Closed** (healthy): no entry in the map, all requests pass through.
//! - **Open**: sidecar call failed, cooldown timer running, all requests rejected.
//! - **Half-open**: cooldown expired, exactly one probe request is allowed through.
//!   Subsequent requests are rejected until the probe completes.
//!
//! Transitions:
//! - Closed → Open: [`mark_unhealthy`] on failure
//! - Open → Half-open: cooldown timer expires (automatic on next [`check_health`])
//! - Half-open → Closed: [`mark_healthy`] on successful probe
//! - Half-open → Open: [`mark_unhealthy`] on probe failure (resets cooldown)
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

/// Per-sandbox breaker state.
struct BreakerEntry {
    /// When the sidecar was marked unhealthy.
    marked_at: Instant,
    /// True when a half-open probe request is in flight. While true, additional
    /// requests are rejected to prevent thundering herd on recovery.
    probing: bool,
}

/// Read-only snapshot of breaker state for a sandbox (no side effects).
pub struct BreakerStatus {
    /// Whether the circuit breaker is currently active (open or half-open).
    pub active: bool,
    /// Seconds remaining in the cooldown period (None if not active).
    pub remaining_secs: Option<u64>,
    /// True when a half-open recovery probe is in flight.
    pub probing: bool,
}

/// Map of sandbox ID -> breaker state.
static UNHEALTHY: Lazy<Mutex<HashMap<String, BreakerEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Tracks the last time GC ran to avoid scanning on every call.
static LAST_GC: Lazy<Mutex<Instant>> = Lazy::new(|| Mutex::new(Instant::now()));

/// Cached cooldown value. Read from `CIRCUIT_BREAKER_COOLDOWN_SECS` env var
/// once on first access, then reused for the lifetime of the process. Avoids
/// calling `getenv(3)` (which acquires a C runtime lock) on every sidecar call.
static COOLDOWN: once_cell::sync::Lazy<u64> = once_cell::sync::Lazy::new(|| {
    std::env::var("CIRCUIT_BREAKER_COOLDOWN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_COOLDOWN_SECS)
});

/// Read the configured cooldown in seconds.
fn cooldown_secs() -> u64 {
    *COOLDOWN
}

/// Check whether `sandbox_id` is healthy enough to accept a request.
///
/// Returns `Ok(())` if:
/// - The sandbox is in the Closed state (no entry), OR
/// - The cooldown has expired and no probe is in flight (transitions to Half-open).
///
/// Returns `Err(SandboxError::Unavailable)` if:
/// - The sandbox is in the Open state (cooldown not yet expired), OR
/// - The sandbox is Half-open with a probe already in flight.
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
            map.retain(|_, entry| entry.marked_at > cutoff);
            *last_gc = Instant::now();
        }
    }

    if let Some(entry) = map.get_mut(sandbox_id) {
        let elapsed = entry.marked_at.elapsed().as_secs();
        if elapsed < cooldown {
            // Open state — cooldown active.
            let remaining = cooldown - elapsed;
            return Err(SandboxError::CircuitBreaker {
                remaining_secs: remaining,
                probing: false,
            });
        }
        // Cooldown expired. If a probe is already in flight, reject.
        if entry.probing {
            return Err(SandboxError::CircuitBreaker {
                remaining_secs: 0,
                probing: true,
            });
        }
        // Transition to half-open: allow this one probe through.
        entry.probing = true;
    }

    Ok(())
}

/// Mark a sandbox as unhealthy (Open state). Subsequent [`check_health`] calls
/// will fail until the cooldown expires. If a half-open probe fails, this
/// resets the cooldown timer.
pub fn mark_unhealthy(sandbox_id: &str) {
    tracing::warn!(sandbox_id, "circuit breaker: marking sidecar unhealthy");
    let mut map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());
    map.insert(
        sandbox_id.to_string(),
        BreakerEntry {
            marked_at: Instant::now(),
            probing: false,
        },
    );
}

/// Mark a sandbox as healthy (Closed state), clearing any cooldown. Call on
/// successful sidecar interaction.
pub fn mark_healthy(sandbox_id: &str) {
    let mut map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());
    map.remove(sandbox_id);
}

/// Alias for [`mark_healthy`] — used when a sandbox is deleted to clean up
/// its circuit-breaker state.
pub fn clear(sandbox_id: &str) {
    mark_healthy(sandbox_id);
}

#[cfg(any(test, feature = "test-utils"))]
pub fn clear_all_for_testing() {
    UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner()).clear();
    *LAST_GC.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
}

/// Read-only query of the breaker state for `sandbox_id`.
///
/// Does **not** trigger GC, state transitions, or any side effects.
pub fn query_status(sandbox_id: &str) -> BreakerStatus {
    let cooldown = cooldown_secs();
    let map = UNHEALTHY.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(sandbox_id) {
        None => BreakerStatus {
            active: false,
            remaining_secs: None,
            probing: false,
        },
        Some(entry) => {
            let elapsed = entry.marked_at.elapsed().as_secs();
            if elapsed < cooldown {
                BreakerStatus {
                    active: true,
                    remaining_secs: Some(cooldown - elapsed),
                    probing: false,
                }
            } else {
                BreakerStatus {
                    active: entry.probing,
                    remaining_secs: Some(0),
                    probing: entry.probing,
                }
            }
        }
    }
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
            msg.contains("cooldown"),
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
    fn test_cooldown_expires_to_half_open() {
        // Insert an entry with an instant far enough in the past that any
        // reasonable cooldown (including the default 30s) would have expired.
        let id = unique_id("cooldown-expires");
        {
            let mut map = UNHEALTHY.lock().unwrap();
            map.insert(
                id.clone(),
                BreakerEntry {
                    marked_at: Instant::now() - std::time::Duration::from_secs(31),
                    probing: false,
                },
            );
        }
        // First call after cooldown: transitions to half-open (probe allowed)
        assert!(
            check_health(&id).is_ok(),
            "should allow probe after cooldown expires"
        );
        // Second call: probe in flight, should be rejected
        let err = check_health(&id);
        assert!(err.is_err(), "should reject while probe is in flight");
        assert!(
            err.unwrap_err().to_string().contains("probe in progress"),
            "error should mention probe in progress"
        );
        // Successful probe: mark_healthy clears completely
        mark_healthy(&id);
        assert!(check_health(&id).is_ok());
    }

    #[test]
    fn test_half_open_probe_failure_resets_cooldown() {
        let id = unique_id("half-open-fail");
        {
            let mut map = UNHEALTHY.lock().unwrap();
            map.insert(
                id.clone(),
                BreakerEntry {
                    marked_at: Instant::now() - std::time::Duration::from_secs(31),
                    probing: false,
                },
            );
        }
        // Probe allowed
        assert!(check_health(&id).is_ok());
        // Probe failed — reset cooldown
        mark_unhealthy(&id);
        // Should be back in open state with fresh cooldown
        let err = check_health(&id);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("cooldown"));
        // Clean up
        clear(&id);
    }

    #[test]
    fn test_gc_removes_stale() {
        let ids: Vec<String> = (0..5).map(|_| unique_id("gc-stale")).collect();
        let stale_instant = Instant::now() - std::time::Duration::from_secs(3600);
        {
            let mut map = UNHEALTHY.lock().unwrap();
            for id in &ids {
                map.insert(
                    id.clone(),
                    BreakerEntry {
                        marked_at: stale_instant,
                        probing: false,
                    },
                );
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

    // ── Phase 2D: Circuit Breaker + Resume Tests ────────────────────────

    #[test]
    fn test_unhealthy_then_mark_healthy_clears() {
        let id = unique_id("resume-clears");
        // Mark unhealthy (simulating sidecar failure)
        mark_unhealthy(&id);
        assert!(
            check_health(&id).is_err(),
            "should be blocked after mark_unhealthy"
        );
        // Simulate successful resume → mark_healthy
        mark_healthy(&id);
        assert!(
            check_health(&id).is_ok(),
            "should be healthy after resume clears breaker"
        );
        // Verify it stays healthy on subsequent checks
        assert!(
            check_health(&id).is_ok(),
            "should remain healthy on repeated checks"
        );
    }

    #[test]
    fn test_breaker_sandbox_scoped_not_global() {
        let id_a = unique_id("scope-a");
        let id_b = unique_id("scope-b");
        // Mark sandbox A unhealthy
        mark_unhealthy(&id_a);
        // Sandbox B should still be healthy
        assert!(
            check_health(&id_b).is_ok(),
            "sandbox B should be healthy when only A is unhealthy"
        );
        assert!(
            check_health(&id_a).is_err(),
            "sandbox A should still be unhealthy"
        );
        // Clean up
        clear(&id_a);
    }
}
