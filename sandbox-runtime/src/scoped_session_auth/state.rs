use super::*;

#[derive(Clone, Debug)]
pub(crate) struct WalletChallengeEntry {
    pub(crate) scope_id: String,
    pub(crate) owner: String,
    pub(crate) wallet_address: String,
    pub(crate) message: String,
    pub(crate) expires_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct SessionEntry {
    /// `Arc<str>` so `resolve_bearer` returns a refcount-bumped handle
    /// instead of heap-allocating + memcpying a fresh `String` on every
    /// authenticated request — the hot path runs at ~1 µs / 10k sessions
    /// and we cannot afford two `String::clone()` calls there.
    pub(crate) scope_id: Arc<str>,
    pub(crate) owner: Arc<str>,
    pub(crate) expires_at: i64,
}

/// GC time gate: a full-map retain runs at most once per this interval, even
/// when the map is below the load-factor threshold. 60 s matches the cadence
/// in `rate_limit::RateLimiter`.
const GC_INTERVAL_MS: u64 = 60_000;

/// GC load-factor gate: when sessions occupy ≥ this fraction of capacity, run
/// GC immediately instead of waiting for the time gate. Caps memory under
/// bursty traffic (e.g. wallet challenge storms) where the time gate would
/// otherwise let the map grow unbounded between sweeps.
const GC_LOAD_FACTOR: f64 = 0.8;

/// `DashMap::len()` and `DashMap::capacity()` walk every shard and acquire a
/// read lock per shard, so they are NOT free on the hot path. We sample the
/// load factor only every `GC_LOAD_SAMPLE_MASK + 1` calls (must be a power
/// of two minus one — used as a bitmask). At 256 the worst-case detection
/// lag is < 1 ms even at 1 Mreq/s, well within the 60 s time gate.
pub(crate) const GC_LOAD_SAMPLE_MASK: u64 = 0xFF; // every 256th call

#[derive(Debug)]
pub(crate) struct ScopedAuthState {
    pub(crate) challenges: DashMap<String, WalletChallengeEntry>,
    pub(crate) sessions: DashMap<String, SessionEntry>,
    /// Unix timestamp in **milliseconds** of the last full GC sweep. Used to
    /// gate GC so `resolve_bearer` stays O(1) on the hot path instead of
    /// O(N). `u64` because Unix time fits comfortably and we never need to
    /// represent values before the epoch; `0` is the "never swept" sentinel.
    pub(crate) last_gc_ms: AtomicU64,
    /// Monotonic counter incremented on every `resolve_bearer` call. Combined
    /// with `GC_LOAD_SAMPLE_MASK` to sample the (locking) DashMap load
    /// factor periodically rather than on every call.
    pub(crate) resolve_calls: AtomicU64,
    /// Wall-clock seconds, refreshed on the sampled cold path (every 256
    /// `resolve_bearer` calls + every write path that already paid for a
    /// syscall). The hot path reads this cached value for the session
    /// expiry check, which avoids the `SystemTime::now()` vDSO call that
    /// dominates the per-call budget at 10k sessions. Stale by at most
    /// 60 s (the GC interval) — small relative to session TTLs measured
    /// in hours.
    pub(crate) cached_now_secs: AtomicI64,
}

impl ScopedAuthState {
    pub(crate) fn new() -> Self {
        Self {
            challenges: DashMap::new(),
            sessions: DashMap::new(),
            last_gc_ms: AtomicU64::new(0),
            resolve_calls: AtomicU64::new(0),
            cached_now_secs: AtomicI64::new(0),
        }
    }

    /// Decide whether GC should run. Two triggers (either is sufficient):
    ///   1. Time gate: `now_ms - last_gc_ms > GC_INTERVAL_MS` — caps how
    ///      stale the map is allowed to get.
    ///   2. Load-factor gate: `sessions.len() / sessions.capacity() >= 0.8`
    ///      — caps how full the map is allowed to get between sweeps.
    ///
    /// Called from write paths unconditionally and from the read path on a
    /// 1/(`GC_LOAD_SAMPLE_MASK` + 1) sample (see `resolve_bearer`).
    pub(crate) fn should_gc(&self, now_ms: u64, last_ms: u64) -> bool {
        if now_ms.saturating_sub(last_ms) > GC_INTERVAL_MS {
            return true;
        }
        load_factor_exceeded(&self.sessions)
    }

    /// Run a full GC sweep when triggered. Thread-safe — uses CAS on
    /// `last_gc_ms` so only one caller does the work, and only when one of
    /// the two GC triggers fires. Read-only (no GC) on the common case.
    ///
    /// `now_secs` is the current wall-clock time in seconds (matches
    /// `expires_at` units on stored entries).
    pub(crate) fn maybe_gc(&self, now_ms: u64, now_secs: i64) {
        let last = self.last_gc_ms.load(Ordering::Relaxed);
        if !self.should_gc(now_ms, last) {
            return;
        }
        // Claim the GC right. If the CAS loses, another thread is running GC —
        // skip our turn. No need to loop or block.
        if self
            .last_gc_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        self.cached_now_secs.store(now_secs, Ordering::Relaxed);
        self.challenges.retain(|_, c| c.expires_at > now_secs);
        self.sessions.retain(|_, s| s.expires_at > now_secs);
    }

    /// Synchronous GC for paths that must observe the latest state (e.g.
    /// capacity checks before insert). Called only on write paths.
    pub(crate) fn gc_now(&self, now_ms: u64, now_secs: i64) {
        self.last_gc_ms.store(now_ms, Ordering::Relaxed);
        self.cached_now_secs.store(now_secs, Ordering::Relaxed);
        self.challenges.retain(|_, c| c.expires_at > now_secs);
        self.sessions.retain(|_, s| s.expires_at > now_secs);
    }
}

/// Current Unix time in milliseconds. Uses `std::time::SystemTime` directly
/// rather than `chrono::Utc::now()` to avoid the DateTime conversion
/// overhead — same vDSO `clock_gettime` syscall, fewer instructions per
/// call. Hot paths read `ScopedAuthState::cached_now_secs` instead of
/// calling this; write paths and the sampled GC trigger use it.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether `(map.len() / map.capacity()) >= GC_LOAD_FACTOR`. Pulled out so
/// the hot-path sampler and the write-path probe share one definition.
/// Both `len` and `capacity` walk every shard and acquire a read lock per
/// shard, so call this sparingly.
fn load_factor_exceeded<K, V>(map: &DashMap<K, V>) -> bool
where
    K: Eq + std::hash::Hash,
{
    let cap = map.capacity();
    if cap == 0 {
        return false;
    }
    // `len()` can briefly exceed `capacity()` between rehashes — saturate
    // by treating any such case as "load high enough, GC".
    map.len() as f64 / cap as f64 >= GC_LOAD_FACTOR
}
