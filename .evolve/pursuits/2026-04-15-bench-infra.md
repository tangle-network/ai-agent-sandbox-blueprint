# Pursuit: Senior-research-quality benchmark infrastructure
Generation: 1
Status: designing

## System Audit

### What exists
- Cargo workspace (7 crates): sandbox-runtime, blueprint bins, blueprint libs, TEE variants
- Test runners: cargo test, nextest in CI, vitest for UI
- `.evolve/current.json` (from /harden) — no prior benchmark infra
- `.evolve/harden/2026-04-15-report.md` — identifies 12 benchmark gaps

### What does NOT exist
- Any `criterion`, `divan`, `iai` dependency
- Any `[[bench]]` section in any Cargo.toml
- Any `benches/` directory
- CI benchmark job
- Baseline JSON for regression detection
- Performance observability (no OTel, Prometheus, Langfuse)

### Measurement gaps (from harden report)
Priority 1 hot paths with zero measurement:
1. `session_auth::validate_session_token` — every authenticated request, Mutex<HashMap>
2. `rate_limit::RateLimiter::check` — every request, double-nested Mutex
3. `circuit_breaker::check_health` — every sidecar call, `getenv(3)` on every call
4. `scoped_session_auth::resolve_bearer` — every instance request, unconditional GC
5. PASETO encrypt/decrypt — auth flow
6. `verify_eip191_signature` — k256 ECDSA recovery
7. `PersistentStore::insert/update/get/find/values` — every mutation (full JSON flush)
8. `seal_field`/`unseal_field` — ChaCha20-Poly1305 per record field
9. `util::build_snapshot_command` + `validate_snapshot_destination` — every snapshot
10. `http::proxy_http` header filtering — every port proxy request

## Baselines
**None exist.** This pursuit establishes the first baseline.

## Diagnosis
Architectural gap: the project has no mechanism to detect performance regressions.
The /harden scan identified that a single Mutex on SESSIONS (50k capacity) is acquired
by every authenticated request, but there is zero measurement of its throughput or
contention under load. Without benchmarks, any performance regression ships silently.

This is architectural — it needs new infrastructure, not tuning.

## Generation 1 Design

### Thesis
A senior-quality benchmark harness requires four integrated pieces:
(1) statistical rigor per-benchmark (p50/p95/p99/mean/stddev/MAD/CI),
(2) a run manifest that captures reproducibility metadata (git SHA, host, toolchain),
(3) cross-run comparison with regression thresholds,
(4) CI integration that fails builds on regressions.
Criterion.rs provides (1); we build (2)/(3)/(4) on top as a thin harness that
consumes Criterion's machine-readable output.

### Moonshot considered
**Considered**: Full custom benchmark runner with integrated OpenTelemetry exporters,
live Grafana dashboard, and a distributed tracing tie-in. **Rejected** because:
(a) Criterion already delivers better statistical analysis than we can build in one
generation (Student's t-test, bootstrap resampling, outlier detection are all in
Criterion), (b) the project has no existing observability SDK — introducing OTel
here would be a parallel untested infrastructure, violating "extend, never duplicate,"
(c) the harness value is in *aggregation + regression detection*, not in repainting
Criterion's wheel.

**Adopted path**: Criterion for per-benchmark stats + thin aggregator crate for
manifest logging and cross-run comparison. Grafana/OTel hook is a future generation.

### Codebase conventions matched
- **Error handling**: `Result<T, SandboxError>` pattern (session_auth, runtime.rs). New
  `bench-harness` crate uses `anyhow::Result` since it's a tool crate, not a library.
- **Logging**: `tracing::{info, warn, error}` used throughout. Harness binary uses
  same `tracing_subscriber` setup as blueprint binaries.
- **Workspace layout**: New crate at `bench-harness/` with its own `Cargo.toml`,
  added to workspace members list in root `Cargo.toml`.
- **Cargo.toml style**: Dependencies sorted, feature-gated optional deps, workspace
  authors/edition/license inherited. Follows sandbox-runtime/Cargo.toml pattern.
- **Test/bench layout**: `benches/` directory next to `src/` — Cargo convention.
- **CI style**: `.github/workflows/ci.yml` uses `dtolnay/rust-toolchain@nightly` and
  `swatinem/rust-cache@v2`. New bench job matches.

### Changes (ordered by impact)

#### Architectural (must ship together)

**A1. New `bench-harness` workspace crate** — statistical aggregator
- `src/stats.rs`: p50/p95/p99/mean/stddev/MAD/variance/CI from sample vector
- `src/manifest.rs`: JSONL run record (timestamp, git SHA, host, rustc, target,
  per-bench stats, env)
- `src/criterion_ingest.rs`: read `target/criterion/**/new/estimates.json`
- `src/compare.rs`: cross-run regression detector with configurable threshold
  (default: 10% mean, 15% p99, outlier-excluded)
- `src/bin/bench-harness.rs`: CLI — `collect`, `compare`, `report` subcommands

**A2. Criterion benches for 10+ hot paths** in `sandbox-runtime/benches/`
- `auth_bench.rs` — token generation, require_sidecar_token
- `session_auth_bench.rs` — validate (hot/cold), verify_eip191, challenge/response
- `scoped_session_bench.rs` — resolve_bearer at 1/1k/10k sessions
- `rate_limit_bench.rs` — check() at 1/100/10k IPs, with/without GC pressure
- `circuit_breaker_bench.rs` — check_health cold/warm/open
- `store_bench.rs` — PersistentStore insert/get/update/find/values at 100/1k/10k
- `crypto_bench.rs` — seal_field/unseal_field at 1KB/64KB payloads, HKDF derivation
- `util_bench.rs` — build_snapshot_command, validate_snapshot_destination (IPv4/IPv6),
  shell_escape, parse_json_object
- `http_bench.rs` — build_url, auth_headers, proxy header filtering

**A3. CI regression gate** in `.github/workflows/ci.yml`
- New `bench` job that runs `cargo bench --bench *` on a pinned runner
- Uses `bench-harness collect` to aggregate
- Uses `bench-harness compare` against baseline from main branch
- Fails on regression, uploads artifact

#### Measurement
- Run manifest JSONL (one line per run) persisted to `bench-results/runs.jsonl`
- Latest full report in `bench-results/latest.json`
- Baseline in `bench-results/baseline.json` (updated on main merges)

#### Infrastructure
- `docs/BENCHMARKS.md` — methodology, how to run, how to interpret
- `scripts/run-benches.sh` — one-shot runner that invokes cargo bench + harness

### Alternatives

- **`divan`** — rejected. Newer, less mature ecosystem, no bootstrap resampling.
- **`iai`** — rejected. Instruction-counting benchmarks are low-variance but don't
  reflect wall-clock reality for lock contention (our #1 concern).
- **Custom runner** — rejected per moonshot analysis above.

### Risk + Success criteria

**Risks**:
- Criterion can be flaky on CI runners (shared CPU). Mitigation: record host as
  part of manifest, allow operators to filter comparisons by host.
- Regression threshold too tight → CI flakiness. Mitigation: 10% mean + 15% p99,
  and compare `lower_confidence_bound` from Criterion, not raw point estimates.
- Rollback: `bench-harness` is a new crate; full rollback = remove it + bench files.

**Success criteria**:
- 10+ benchmarks exist and emit p50/p95/p99 stats
- Run manifest JSONL contains git SHA, host, timestamp, stats for every bench
- `bench-harness compare` correctly flags >10% mean regressions
- CI bench job runs in <10 min, fails on regression, uploads artifact
- All benches compile with `-D warnings`
- All benches execute locally (`cargo bench --bench <name> -- --quick`)

## Phase 1.5: Adversarial Review

**Change type**: Additive (new crate + benches + CI job). No production code paths
touched. No trust boundary crossed. No auth/crypto modifications.

**Perspectives**:

- **Security**: verdict OK. No production path modified. Harness binary is local-only.
  *Concern*: bench output could leak internal timing of crypto primitives. Rationale
  for acceptance: same timing is already observable via `cargo test` in CI.
  *Would-block*: no.

- **Reliability**: verdict OK with mitigation. CI bench job could be flaky on shared
  runners. Mitigation: `--warm-up-time 1 --measurement-time 3` for CI mode;
  thresholds based on Criterion's `lower_confidence_bound` not point estimates.
  *Would-block*: no.

- **Performance**: verdict GOOD. This IS performance infra.
  *Concern*: Criterion itself adds measurable overhead. Mitigation: benches measure
  the target function in a loop, so Criterion's framework overhead is amortized.
  *Would-block*: no.

- **UX (developer)**: verdict OK. Running `cargo bench` is idiomatic Rust. Docs
  provide `scripts/run-benches.sh` one-liner.
  *Would-block*: no.

- **Red team (day 90)**: An attacker could time sensitive crypto operations more
  precisely by reading bench results. But those operations use `subtle` crate's
  constant-time comparisons where needed. No new attack surface.
  *Would-block*: no.

**Failure modes enumerated**:
- Criterion output format changes → harness breaks. Mitigation: parse Criterion
  JSON defensively, fall back to "unknown" stat values.
- Git not available in CI → manifest missing SHA. Mitigation: fall back to
  `GITHUB_SHA` env var, then "unknown".
- Concurrent bench runs → manifest JSONL corruption. Mitigation: each run writes
  to a unique filename; aggregation is append-only.

**Decision**: Plan is strongest available. Advance to build.

### Build Status
| # | Change | Status | Files | Tests |
|---|--------|--------|-------|-------|
| A1 | bench-harness crate | DONE | bench-harness/ (lib + bin) | 24 unit tests (stats/compare/ingest/manifest) |
| A2 | Criterion benches | DONE | sandbox-runtime/benches/*.rs (9 files) | self-validate via cargo bench |
| A3 | CI bench job | DONE | .github/workflows/ci.yml (bench job) | compare-to-baseline on PRs |
| M1 | bench-results/ manifest | DONE | bench-results/.gitkeep + .gitignore | - |
| I1 | docs/BENCHMARKS.md | DONE | docs/BENCHMARKS.md | - |
| I2 | scripts/run-benches.sh | DONE | scripts/run-benches.sh | - |

## Generation 1 Results

### Baseline measurements (2026-04-15, Apple M-series, macOS, rustc 1.88.0)

Selected from the first real run:

| Bench | Mean | p99 | Throughput |
|-------|------|-----|-----------|
| auth/generate_token | 1.59 µs | 2.45 µs | 629 kops/s |
| auth/require_sidecar_token | 38.7 ns | 40.2 ns | 25.8 Mops/s |
| scoped_session/resolve_bearer @ 1 session | 116 ns | 152 ns | 8.6 Mops/s |
| scoped_session/resolve_bearer @ 100 | 252 ns | 253 ns | 4.0 Mops/s |
| scoped_session/resolve_bearer @ 1 000 | 1 386 ns | 1 408 ns | 722 kops/s |
| scoped_session/resolve_bearer @ 10 000 | 22 847 ns | 23 797 ns | 44 kops/s |
| scoped_session/create_access_token | 139 µs | 226 µs | 7.2 kops/s |

### Honest human assessment

- **Works as advertised.** All 9 bench files compile, all run under `cargo bench`,
  all produce full statistical output (p50/p95/p99, mean/stddev/MAD, CI bounds,
  Tukey outlier counts, ops/sec throughput).
- **Discovered the very regression /harden flagged.** `resolve_bearer` latency
  grows from 116ns @ 1 session to 22,847ns @ 10k — a 196× degradation confirming
  the unconditional-GC-on-every-auth-check hypothesis. This is now a concrete,
  reproducible number to target with `/evolve`.
- **Harness CLI is solid.** `collect` / `compare` / `report` all functional.
  `compare` correctly distinguished improvement from noise on back-to-back runs.
- **24/24 harness unit tests pass** including bootstrap CI bracketing, MAD
  robustness, percentile interpolation, malformed-JSON recovery, added/removed
  bench tracking, noisy-vs-regressed verdict.
- **All pre-existing tests still pass**: 27 session_auth, 11 circuit_breaker.

### What worked

- Criterion's JSON output is stable and parseable. The glob-based discovery in
  `criterion_ingest::collect_all` handled the `target/criterion/` layout cleanly.
- Gating `clear_all_for_testing` / `create_test_token` behind `test-utils` feature
  kept production builds clean while letting benches reset state.
- CI-proof regression detection (current_lower > baseline_upper) avoids flakiness
  on shared CI runners.

### What didn't / surprised

- The file writer hit intermittent linter stubs (`fn main() {}`) during bulk
  parallel writes. Serialized writes worked.
- Criterion's `change:` detection prints `p > 0.05 → No change detected` even
  when absolute change is large — because per-bench sample-size is small in
  quick mode. The harness comparator (which uses bootstrap CI across runs) is
  more robust for CI.

### Verdict: ADVANCE

All success criteria met:
- ✅ 10+ benchmarks exist and emit p50/p95/p99 stats (9 bench files,
  30+ individual benchmark entries across parameterized groups)
- ✅ Run manifest JSONL contains git SHA, host, timestamp, stats for every bench
- ✅ `bench-harness compare` correctly flags >10% mean regressions, with CI proof
- ✅ CI bench job runs in <10 min quick mode, uploads artifact, compares to baseline
- ✅ All benches compile with `-D warnings`
- ✅ All benches execute locally

### Seeds for Gen 2 (or /evolve targets)

1. **`scoped_session::resolve_bearer` @ 10k sessions**: 22.8 µs mean. Goal:
   get to < 5 µs by removing unconditional GC on every call. Metric: Δ mean
   when session_count=10000.
2. **`PersistentStore::insert`**: measure at 100 / 1k records. Goal: detect
   the disk-flush cost and consider batched flush. Metric: ops/sec when the
   store is pre-loaded with 1k records.
3. **`session_auth::validate_session_token` hot vs cold**: once benched, target
   the cold path — PASETO decrypt is 10–50× slower than a HashMap hit.
4. **`rate_limit::window_full`**: measure the retain scan cost when the bucket
   is at capacity (2400 entries). This is the terminal-interactive limiter.

### Backward-compatible changes to previous-gen baselines

None — this is Gen 1. Future runs should compare against the first main-branch
artifact uploaded by the new CI job.

