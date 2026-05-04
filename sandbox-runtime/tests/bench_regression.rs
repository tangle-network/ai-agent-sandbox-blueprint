//! CI-gating regression test for `scoped_session_auth::resolve_bearer`.
//!
//! Background. The pursuit doc `.evolve/pursuits/2026-04-15-bench-infra.md`
//! recorded the `resolve_bearer` baseline on Apple M-series + macOS:
//!
//!   |   sessions  | mean (ns) |
//!   |    1        |       116 |
//!   |   100       |       252 |
//!   |   1 000     |     1 386 |
//!   |  10 000     |    22 847 |  ← 196× degradation, hit by every auth check
//!
//! Cause: an unconditional full-map GC on every call. The production fix
//! switches to a DashMap and gates GC on (load-factor ≥ 0.8) OR
//! (60 s elapsed). Target: < 1 µs at 10 k sessions on equivalent hardware.
//!
//! This test is the CI gate. Criterion benches give precise per-bench stats
//! in `target/criterion/`, but Criterion does not fail builds on its own.
//! We compute mean wall-clock time over a fixed iteration count and panic
//! if it exceeds a generous CI threshold (1.5 µs) — that gives us headroom
//! over the 1 µs target while still catching any regression that
//! reintroduces O(N) behaviour. CI runners are noisy; the threshold is
//! tuned to keep false positives rare while still flagging the documented
//! 22.8 µs regression class.
//!
//! Threshold rationale:
//! - Target: 1 µs (5× slower than the 200 ns DashMap baseline measured in
//!   the post-evolve run).
//! - CI gate: 1.5 µs (target × 1.5 to absorb shared-runner jitter).
//! - Regression class we must catch: 22.8 µs+ (15× the gate).
//!
//! If this test starts failing, do NOT raise the threshold — go check
//! `scoped_session_auth::ScopedAuthState::should_gc` for an accidental
//! O(N) path or a deadlock that forces a write under read.

use std::time::Instant;

use sandbox_runtime::scoped_session_auth::{
    ScopedAuthConfig, ScopedAuthMode, ScopedAuthResource, ScopedAuthService,
};

const SESSION_COUNT: usize = 10_000;
const ITERATIONS: usize = 100_000;
const THRESHOLD_NS_PER_CALL: u128 = 1_500;

#[test]
fn resolve_bearer_stays_under_threshold_at_10k_sessions() {
    let service = ScopedAuthService::new(ScopedAuthConfig {
        access_token: Some("shared-token".to_string()),
        max_sessions: SESSION_COUNT * 2,
        max_challenges: 100_000,
        ..ScopedAuthConfig::default()
    });

    let mut tokens = Vec::with_capacity(SESSION_COUNT);
    for i in 0..SESSION_COUNT {
        let resource = ScopedAuthResource {
            scope_id: format!("inst-{i}"),
            owner: format!("0x{:040x}", i + 1),
            auth_mode: ScopedAuthMode::AccessToken,
        };
        let session = service
            .create_access_token_session(&resource, "shared-token")
            .expect("create session");
        tokens.push(session.token);
    }

    // Warm-up: prime caches and ensure GC has run once so the first iteration
    // doesn't carry the cold-path penalty into the measured mean.
    for token in tokens.iter().take(1_000) {
        let _ = service.resolve_bearer(token);
    }

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let token = &tokens[i % tokens.len()];
        let claims = service.resolve_bearer(token);
        // Defeat dead-code elimination on the resolved value.
        std::hint::black_box(claims);
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let mean_ns = elapsed_ns / ITERATIONS as u128;

    assert!(
        mean_ns <= THRESHOLD_NS_PER_CALL,
        "resolve_bearer mean {mean_ns} ns exceeds CI threshold {THRESHOLD_NS_PER_CALL} ns at \
         {SESSION_COUNT} sessions. Baseline pre-evolve was 22 847 ns (BTreeMap+unconditional GC); \
         current code likely regressed onto a write-locked or O(N) path. \
         See sandbox-runtime/src/scoped_session_auth.rs."
    );
    eprintln!(
        "resolve_bearer @ {SESSION_COUNT} sessions: mean = {mean_ns} ns/call \
         (threshold {THRESHOLD_NS_PER_CALL} ns)"
    );
}
