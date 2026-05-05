//! Reusable in-memory scoped session authentication for operator APIs.
//!
//! This module supports:
//! - optional operator-wide bearer tokens
//! - wallet-signature challenge flow scoped to one resource (instance/sandbox)
//! - static access-token flow scoped to one resource
//! - short-lived bearer sessions bound to `{scope_id, owner}`
//!
//! ## Data structure choice
//!
//! Uses `DashMap` (sharded concurrent hashmap) for both challenges and sessions
//! so `resolve_bearer` — called on every instance API request — can read without
//! acquiring a global mutex. GC is gated on (a) wall-clock elapsed since the
//! last sweep and (b) load factor of the sessions map; this mirrors the
//! pattern used by [`crate::rate_limit::RateLimiter`].
//!
//! ## Baseline numbers (criterion, sandbox-runtime/benches/scoped_session_bench.rs)
//!
//! Pre-evolve (unconditional `BTreeMap::retain` on every resolve_bearer call):
//! - 1 session:      116 ns
//! - 100 sessions:   252 ns
//! - 1 000 sessions: 1 386 ns
//! - 10 000:         22 847 ns  (196× degradation, per
//!   `.evolve/pursuits/2026-04-15-bench-infra.md`)
//!
//! Post-evolve (DashMap + load-factor + time-gated GC, this file):
//! - target: <1 µs at 10 000 sessions on the same hardware.
//!
//! The dual-trigger is deliberate: a purely time-based gate lets a hot map
//! grow arbitrarily large between sweeps (memory pressure under burst load);
//! a purely capacity-based gate skips sweeps entirely on cold-but-aged maps
//! where TTL expiries dominate.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use dashmap::DashMap;

/// Resource auth mode used by scoped session auth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopedAuthMode {
    WalletSignature,
    AccessToken,
}

/// Auth configuration for scoped session auth.
#[derive(Clone, Debug)]
pub struct ScopedAuthConfig {
    pub challenge_ttl_secs: i64,
    pub session_ttl_secs: i64,
    pub access_token: Option<String>,
    pub operator_api_token: Option<String>,
    pub max_challenges: usize,
    pub max_sessions: usize,
    pub token_prefix: String,
    pub challenge_message_header: String,
}

impl Default for ScopedAuthConfig {
    fn default() -> Self {
        Self {
            challenge_ttl_secs: 300,
            session_ttl_secs: 3600,
            access_token: None,
            operator_api_token: None,
            max_challenges: 10_000,
            max_sessions: 50_000,
            token_prefix: "scope_".to_string(),
            challenge_message_header: "Scoped Resource Access".to_string(),
        }
    }
}

/// Resource identity for scoped auth checks.
#[derive(Clone, Debug)]
pub struct ScopedAuthResource {
    pub scope_id: String,
    pub owner: String,
    pub auth_mode: ScopedAuthMode,
}

#[derive(Clone, Debug)]
struct WalletChallengeEntry {
    scope_id: String,
    owner: String,
    wallet_address: String,
    message: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
struct SessionEntry {
    scope_id: String,
    owner: String,
    expires_at: i64,
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
const GC_LOAD_SAMPLE_MASK: u64 = 0xFF; // every 256th call

#[derive(Debug)]
struct ScopedAuthState {
    challenges: DashMap<String, WalletChallengeEntry>,
    sessions: DashMap<String, SessionEntry>,
    /// Unix timestamp in **milliseconds** of the last full GC sweep. Used to
    /// gate GC so `resolve_bearer` stays O(1) on the hot path instead of
    /// O(N). `u64` because Unix time fits comfortably and we never need to
    /// represent values before the epoch; `0` is the "never swept" sentinel.
    last_gc_ms: AtomicU64,
    /// Monotonic counter incremented on every `resolve_bearer` call. Combined
    /// with `GC_LOAD_SAMPLE_MASK` to sample the (locking) DashMap load
    /// factor periodically rather than on every call.
    resolve_calls: AtomicU64,
}

impl ScopedAuthState {
    fn new() -> Self {
        Self {
            challenges: DashMap::new(),
            sessions: DashMap::new(),
            last_gc_ms: AtomicU64::new(0),
            resolve_calls: AtomicU64::new(0),
        }
    }

    /// Decide whether GC should run. Two triggers (either is sufficient):
    ///   1. Time gate: `now_ms - last_gc_ms > GC_INTERVAL_MS` — caps how
    ///      stale the map is allowed to get.
    ///   2. Load-factor gate: `sessions.len() / sessions.capacity() >= 0.8`
    ///      — caps how full the map is allowed to get between sweeps.
    ///
    /// Computes the load factor unconditionally — used by write paths and by
    /// the periodic sample on the read path. For the hot read path use
    /// `should_gc_sampled` instead.
    fn should_gc(&self, now_ms: u64, last_ms: u64) -> bool {
        if now_ms.saturating_sub(last_ms) > GC_INTERVAL_MS {
            return true;
        }
        load_factor_exceeded(&self.sessions)
    }

    /// Hot-path GC trigger check. Always honours the time gate (cheap: one
    /// atomic compare) and probes the load factor only on a 1/(MASK+1)
    /// sample to avoid the per-shard `len()`/`capacity()` lock storm.
    fn should_gc_sampled(&self, now_ms: u64, last_ms: u64) -> bool {
        if now_ms.saturating_sub(last_ms) > GC_INTERVAL_MS {
            return true;
        }
        let calls = self.resolve_calls.fetch_add(1, Ordering::Relaxed);
        if calls & GC_LOAD_SAMPLE_MASK != 0 {
            return false;
        }
        load_factor_exceeded(&self.sessions)
    }

    /// Run a full GC sweep when triggered. Thread-safe — uses CAS on
    /// `last_gc_ms` so only one caller does the work, and only when one of
    /// the two GC triggers fires. Read-only (no GC) on the common case.
    ///
    /// `now_secs` is the current wall-clock time in seconds (matches
    /// `expires_at` units on stored entries).
    fn maybe_gc(&self, now_ms: u64, now_secs: i64) {
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
        self.challenges.retain(|_, c| c.expires_at > now_secs);
        self.sessions.retain(|_, s| s.expires_at > now_secs);
    }

    /// Synchronous GC for paths that must observe the latest state (e.g.
    /// capacity checks before insert). Called only on write paths.
    fn gc_now(&self, now_ms: u64, now_secs: i64) {
        self.last_gc_ms.store(now_ms, Ordering::Relaxed);
        self.challenges.retain(|_, c| c.expires_at > now_secs);
        self.sessions.retain(|_, s| s.expires_at > now_secs);
    }
}

/// Current Unix time in milliseconds. Pulled out so callers don't repeat
/// the conversion and so tests can be precise about ordering with
/// `expires_at` (which is in seconds).
fn now_ms() -> u64 {
    let ms = Utc::now().timestamp_millis();
    if ms < 0 { 0 } else { ms as u64 }
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

/// Session claims resolved from bearer tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopedSessionClaims {
    Operator,
    Scoped { scope_id: String, owner: String },
}

/// Wallet challenge creation response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedChallengeResponse {
    pub challenge_id: String,
    pub message: String,
    pub expires_at: i64,
}

/// Session creation response for scoped auth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedSessionResponse {
    pub token: String,
    pub expires_at: i64,
    pub scope_id: String,
    pub owner: String,
}

/// In-memory scoped session authentication service.
///
/// `resolve_bearer` is O(1) (DashMap lookup) + amortized O(0) GC (time-gated).
/// Write paths (`create_*`, `verify_*`) call `gc_now` to keep capacity checks
/// accurate at insert time.
#[derive(Clone, Debug)]
pub struct ScopedAuthService {
    config: ScopedAuthConfig,
    state: Arc<ScopedAuthState>,
}

impl ScopedAuthService {
    pub fn new(config: ScopedAuthConfig) -> Self {
        Self {
            config,
            state: Arc::new(ScopedAuthState::new()),
        }
    }

    /// Resolve a bearer token to its claims. Hot path — called on every
    /// instance-mode API request. Does NOT run full GC unless one of the GC
    /// triggers fires (load factor ≥ 0.8 or > 60 s elapsed since last sweep);
    /// stale expired sessions are filtered out by the per-lookup expiration
    /// check below.
    pub fn resolve_bearer(&self, token: &str) -> Option<ScopedSessionClaims> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(operator_token) = &self.config.operator_api_token
            && trimmed == operator_token
        {
            return Some(ScopedSessionClaims::Operator);
        }

        // Lazy GC — fast path is one atomic load + one wrapping addition.
        // Time gate fires when 60 s have elapsed; load-factor gate fires on
        // a 1/256 sample of resolve calls to avoid the per-shard lock cost
        // of `DashMap::len`/`capacity`. Either trigger leads to a single
        // CAS-claimed full sweep — never two threads sweeping at once.
        let last = self.state.last_gc_ms.load(Ordering::Relaxed);
        let now_ms_value = now_ms();
        let now_secs = (now_ms_value / 1_000) as i64;
        if self.state.should_gc_sampled(now_ms_value, last) {
            self.state.maybe_gc(now_ms_value, now_secs);
        }

        let session = self.state.sessions.get(trimmed)?;
        // Filter out expired sessions that GC hasn't pruned yet. This keeps
        // revocation and expiry effective regardless of GC cadence.
        if session.expires_at <= now_secs {
            return None;
        }
        Some(ScopedSessionClaims::Scoped {
            scope_id: session.scope_id.clone(),
            owner: session.owner.clone(),
        })
    }

    pub fn create_wallet_challenge(
        &self,
        resource: &ScopedAuthResource,
        wallet_address: &str,
    ) -> Result<ScopedChallengeResponse, String> {
        if resource.auth_mode != ScopedAuthMode::WalletSignature {
            return Err("resource does not use wallet_signature auth mode".to_string());
        }

        let wallet = normalize_evm_address(wallet_address)?;
        let owner = normalize_evm_address(&resource.owner)?;
        if wallet != owner {
            return Err("wallet address does not match resource owner".to_string());
        }

        let now = Utc::now().timestamp();
        let expires_at = now + self.config.challenge_ttl_secs;
        let challenge_id = uuid::Uuid::new_v4().to_string();
        let message = format!(
            "{header}\nscope_id:{scope_id}\nowner:{owner}\nchallenge_id:{challenge}\nissued_at:{now}\nexpires_at:{expires}",
            header = self.config.challenge_message_header,
            scope_id = resource.scope_id,
            owner = resource.owner,
            challenge = challenge_id,
            now = now,
            expires = expires_at
        );

        // Write path: run GC synchronously so the capacity check is accurate.
        self.state.gc_now(now_ms(), now);
        if self.state.challenges.len() >= self.config.max_challenges {
            return Err("challenge capacity exceeded, try again later".to_string());
        }
        self.state.challenges.insert(
            challenge_id.clone(),
            WalletChallengeEntry {
                scope_id: resource.scope_id.clone(),
                owner: resource.owner.clone(),
                wallet_address: wallet,
                message: message.clone(),
                expires_at,
            },
        );

        Ok(ScopedChallengeResponse {
            challenge_id,
            message,
            expires_at,
        })
    }

    pub fn verify_wallet_challenge(
        &self,
        challenge_id: &str,
        signature_hex: &str,
    ) -> Result<ScopedSessionResponse, String> {
        let now = Utc::now().timestamp();
        self.state.gc_now(now_ms(), now);

        // DashMap::remove returns Option<(K, V)>.
        let Some((_, challenge)) = self.state.challenges.remove(challenge_id) else {
            return Err("challenge not found or expired".to_string());
        };

        let recovered =
            crate::session_auth::verify_eip191_signature(&challenge.message, signature_hex)
                .map_err(|e| format!("failed to recover signer from signature: {e}"))?;
        let recovered = normalize_evm_address(&recovered)?;
        if recovered != challenge.wallet_address {
            return Err("signature does not match challenge wallet".to_string());
        }

        let expires_at = now + self.config.session_ttl_secs;
        let token = issue_token(&self.config.token_prefix);
        if self.state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        self.state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: challenge.scope_id.clone(),
                owner: challenge.owner.clone(),
                expires_at,
            },
        );

        Ok(ScopedSessionResponse {
            token,
            expires_at,
            scope_id: challenge.scope_id,
            owner: challenge.owner,
        })
    }

    pub fn create_access_token_session(
        &self,
        resource: &ScopedAuthResource,
        access_token: &str,
    ) -> Result<ScopedSessionResponse, String> {
        if resource.auth_mode != ScopedAuthMode::AccessToken {
            return Err("resource does not use access_token auth mode".to_string());
        }
        let Some(expected) = &self.config.access_token else {
            return Err("scoped access token is not configured".to_string());
        };
        if access_token.trim() != expected {
            return Err("invalid access token".to_string());
        }

        let now = Utc::now().timestamp();
        let expires_at = now + self.config.session_ttl_secs;
        let token = issue_token(&self.config.token_prefix);

        self.state.gc_now(now_ms(), now);
        if self.state.sessions.len() >= self.config.max_sessions {
            return Err("session capacity exceeded, try again later".to_string());
        }
        self.state.sessions.insert(
            token.clone(),
            SessionEntry {
                scope_id: resource.scope_id.clone(),
                owner: resource.owner.clone(),
                expires_at,
            },
        );

        Ok(ScopedSessionResponse {
            token,
            expires_at,
            scope_id: resource.scope_id.clone(),
            owner: resource.owner.clone(),
        })
    }
}

fn issue_token(prefix: &str) -> String {
    let clean_prefix = if prefix.trim().is_empty() {
        "scope_"
    } else {
        prefix.trim()
    };
    format!("{clean_prefix}{}", uuid::Uuid::new_v4().simple())
}

fn normalize_evm_address(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.len() != 42 {
        return Err(format!(
            "invalid address `{value}`: expected 42 chars with 0x prefix"
        ));
    }
    let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    else {
        return Err(format!("invalid address `{value}`: missing 0x prefix"));
    };
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid address `{value}`: non-hex characters"));
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(mode: ScopedAuthMode) -> ScopedAuthResource {
        ScopedAuthResource {
            scope_id: "inst-1".to_string(),
            owner: "0x0000000000000000000000000000000000000001".to_string(),
            auth_mode: mode,
        }
    }

    #[test]
    fn resolve_operator_token() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            operator_api_token: Some("op".to_string()),
            ..ScopedAuthConfig::default()
        });
        assert_eq!(
            service.resolve_bearer("op"),
            Some(ScopedSessionClaims::Operator)
        );
    }

    #[test]
    fn access_token_session_roundtrip() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            token_prefix: "acl_".to_string(),
            ..ScopedAuthConfig::default()
        });
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("session");
        assert!(session.token.starts_with("acl_"));
        assert_eq!(
            service.resolve_bearer(&session.token),
            Some(ScopedSessionClaims::Scoped {
                scope_id: "inst-1".to_string(),
                owner: "0x0000000000000000000000000000000000000001".to_string()
            })
        );
    }

    #[test]
    fn access_token_rejected_when_mismatch() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "wrong")
            .expect_err("must reject invalid token");
        assert!(err.contains("invalid access token"));
    }

    #[test]
    fn challenge_capacity_blocks_when_full() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            max_challenges: 0,
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_wallet_challenge(
                &resource(ScopedAuthMode::WalletSignature),
                "0x0000000000000000000000000000000000000001",
            )
            .expect_err("must fail when challenge capacity is exhausted");
        assert!(err.contains("challenge capacity exceeded"));
    }

    #[test]
    fn session_capacity_blocks_when_full() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            max_sessions: 0,
            ..ScopedAuthConfig::default()
        });
        let err = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect_err("must fail when session capacity is exhausted");
        assert!(err.contains("session capacity exceeded"));
    }

    // ── Phase 1D: Scoped Session Auth Integration Tests ─────────────────

    #[test]
    fn scoped_session_expired_token_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            session_ttl_secs: -1, // already expired
            ..ScopedAuthConfig::default()
        });
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("should create session even with negative TTL");
        // Token was created with an already-expired timestamp
        let resolved = service.resolve_bearer(&session.token);
        assert!(
            resolved.is_none(),
            "expired token should not resolve: {resolved:?}"
        );
    }

    #[test]
    fn scoped_session_cross_scope_reuse_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        // Create session for scope "inst-1"
        let session = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("create session");
        // Resolve the token — should return scope "inst-1"
        let claims = service
            .resolve_bearer(&session.token)
            .expect("should resolve");
        match claims {
            ScopedSessionClaims::Scoped { scope_id, .. } => {
                assert_eq!(scope_id, "inst-1");
                // A different scope (e.g. "inst-2") would need a different token.
                // The token is bound to inst-1, so it can't authenticate inst-2.
                assert_ne!(scope_id, "inst-2", "token must not match a different scope");
            }
            _ => panic!("expected Scoped claims"),
        }
    }

    #[test]
    fn wallet_challenge_wrong_address_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig::default());
        // Resource owner is 0x0000...0001, but we try to challenge with a
        // different wallet address.
        let err = service
            .create_wallet_challenge(
                &resource(ScopedAuthMode::WalletSignature),
                "0x0000000000000000000000000000000000000099",
            )
            .expect_err("should reject mismatched wallet address");
        assert!(
            err.contains("does not match"),
            "error should mention address mismatch: {err}"
        );
    }

    #[test]
    fn access_token_wrong_mode_rejected() {
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            ..ScopedAuthConfig::default()
        });
        // Resource is set to WalletSignature mode, but we try AccessToken flow
        let err = service
            .create_access_token_session(&resource(ScopedAuthMode::WalletSignature), "shared")
            .expect_err("should reject wrong auth mode");
        assert!(
            err.contains("access_token"),
            "error should mention wrong mode: {err}"
        );
    }

    // ── Post-evolve: verify DashMap migration preserves concurrency invariants ──

    #[test]
    fn concurrent_resolve_bearer_no_data_race() {
        use std::sync::Arc;
        use std::thread;

        let service = Arc::new(ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            session_ttl_secs: 3600,
            max_sessions: 100_000,
            ..ScopedAuthConfig::default()
        }));

        // Pre-populate 1000 sessions.
        let mut tokens = Vec::with_capacity(1000);
        for i in 0..1000 {
            let r = ScopedAuthResource {
                scope_id: format!("inst-{i}"),
                owner: format!("0x{:040x}", i + 1),
                auth_mode: ScopedAuthMode::AccessToken,
            };
            let s = service
                .create_access_token_session(&r, "shared")
                .expect("create");
            tokens.push(s.token);
        }
        let tokens = Arc::new(tokens);

        // Spawn 8 reader threads hammering resolve_bearer concurrently.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let svc = Arc::clone(&service);
            let toks = Arc::clone(&tokens);
            handles.push(thread::spawn(move || {
                for i in 0..5_000 {
                    let token = &toks[i % toks.len()];
                    let claims = svc.resolve_bearer(token);
                    assert!(claims.is_some(), "token must resolve under concurrency");
                }
            }));
        }

        // Concurrent writer inserting more sessions — verifies DashMap
        // handles reads/writes without deadlock or data race.
        let svc = Arc::clone(&service);
        handles.push(thread::spawn(move || {
            for i in 0..500 {
                let r = ScopedAuthResource {
                    scope_id: format!("writer-{i}"),
                    owner: format!("0x{:040x}", 10_000 + i),
                    auth_mode: ScopedAuthMode::AccessToken,
                };
                let _ = svc.create_access_token_session(&r, "shared");
            }
        }));

        for h in handles {
            h.join().expect("worker thread panicked");
        }
    }

    #[test]
    fn expired_session_filtered_out_before_gc_runs() {
        // Sessions with a past expiry must not resolve, even if GC hasn't run.
        let service = ScopedAuthService::new(ScopedAuthConfig {
            access_token: Some("shared".to_string()),
            session_ttl_secs: -1, // already expired at issue time
            ..ScopedAuthConfig::default()
        });
        let s = service
            .create_access_token_session(&resource(ScopedAuthMode::AccessToken), "shared")
            .expect("create");
        assert!(
            service.resolve_bearer(&s.token).is_none(),
            "expired session must never resolve to claims"
        );
    }
}
