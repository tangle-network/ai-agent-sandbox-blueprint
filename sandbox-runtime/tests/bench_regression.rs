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
//! lives in `scoped_session_auth.rs`: DashMap, time-cached wall-clock,
//! sampled GC, `Arc<str>` return values. Target: < 1 µs at 10 k sessions
//! on equivalent dedicated hardware.
//!
//! ## Threshold methodology
//!
//! This test runs in two materially different environments:
//!
//! - Dedicated dev hardware (Apple M-series, modern x86): ~700–900 ns/call
//!   in `--release`. Comfortable headroom over a 1.5 µs ceiling.
//! - GitHub Actions `ubuntu-latest` shared runners under `cargo tarpaulin`:
//!   ~1,800–2,200 ns/call. The instrumentation overhead and shared-CPU
//!   contention add a real ~2× tax that no amount of code optimization
//!   inside `resolve_bearer` can recover.
//!
//! Rather than special-casing the build, we **calibrate the threshold
//! against the host** using a cheap-but-representative operation
//! (`Instant::now()`, ~25 ns on Apple Silicon, ~60–80 ns on shared CI).
//! The threshold is a multiple of that calibration result, so a code
//! regression that's slower than the hardware can explain still fails
//! the gate everywhere, but normal hardware-and-environment variance
//! doesn't.
//!
//! Regression class we must still catch: 22.8 µs+ (the pre-evolve
//! BTreeMap-with-unconditional-GC behaviour) — orders of magnitude over
//! the calibrated ceiling on every host.
//!
//! If this test starts failing, first check `scoped_session_auth::resolve_bearer`
//! for a real regression — an accidental syscall on the hot path, an
//! unconditional clone, or a regressed locking pattern — by confirming the
//! measured mean is ~1 µs, not the tens-of-µs pre-evolve class. Only widen
//! `THRESHOLD_NS_MULTIPLIER` once the hot path is confirmed unchanged (as it
//! was when the multiplier moved 30→100: host-to-host ratio variance on
//! untouched code, not a regression).

use std::time::Instant;

use sandbox_runtime::scoped_session_auth::{
    ScopedAuthConfig, ScopedAuthMode, ScopedAuthResource, ScopedAuthService,
};

const SESSION_COUNT: usize = 10_000;
const ITERATIONS: usize = 100_000;
const CALIBRATION_ITERATIONS: usize = 1_000_000;

/// Per-call budget expressed as a multiple of the calibration unit-cost.
/// The calibration op (`Instant::now`, a vDSO `clock_gettime`) is a proxy for
/// raw CPU speed, but `resolve_bearer`'s cost is a HashMap lookup + token
/// validation — so the measured *ratio* varies with cache / branch-prediction
/// across microarchitectures: 28–40× in practice (dev ~34×, shared CI ~31×).
/// 30× left no headroom for that spread and false-positived on hosts where the
/// hot path was unchanged. 100× absorbs the real-world ratio while still
/// catching the regression class this guard exists for: the pre-evolve
/// BTreeMap + unconditional-GC path at 22.8 µs is ~600× the calibration unit —
/// 6× above this ceiling on every host, so a genuine regression still fails
/// everywhere. See the module docstring before widening further.
const THRESHOLD_NS_MULTIPLIER: u128 = 100;

#[test]
fn resolve_bearer_stays_under_threshold_at_10k_sessions() {
    // ── Calibrate against the host. `Instant::now()` is a vDSO
    //    `clock_gettime` call — the same syscall family `resolve_bearer`'s
    //    cold path uses, and dominated by the same CPU / cache costs.
    let cal_start = Instant::now();
    for _ in 0..CALIBRATION_ITERATIONS {
        std::hint::black_box(Instant::now());
    }
    let cal_ns = (cal_start.elapsed().as_nanos() / CALIBRATION_ITERATIONS as u128).max(1);
    let threshold_ns = cal_ns * THRESHOLD_NS_MULTIPLIER;

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
        mean_ns <= threshold_ns,
        "resolve_bearer mean {mean_ns} ns exceeds calibrated CI threshold {threshold_ns} ns \
         (calibration {cal_ns} ns/op × {THRESHOLD_NS_MULTIPLIER}×) at {SESSION_COUNT} sessions. \
         Baseline pre-evolve was 22 847 ns (BTreeMap + unconditional GC). Likely cause: an \
         accidental `SystemTime::now()` syscall on the hot path, an unconditional clone of an \
         owned String, or a regressed locking pattern. See sandbox-runtime/src/scoped_session_auth.rs."
    );
    eprintln!(
        "resolve_bearer @ {SESSION_COUNT} sessions: mean = {mean_ns} ns/call \
         (calibrated threshold {threshold_ns} ns, host {cal_ns} ns/op)"
    );
}
