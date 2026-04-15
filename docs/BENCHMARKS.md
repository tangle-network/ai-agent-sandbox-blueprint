# Benchmarks

This project maintains a first-class benchmark suite covering the hot paths of
`sandbox-runtime`. Every authenticated API request traverses code in
`session_auth`, `rate_limit`, and `circuit_breaker`; every storage mutation
flushes the JSON store to disk; every sidecar call constructs URLs and builds
auth headers. These benches measure that cost and detect regressions in CI.

## Running locally

```bash
# Full run (takes a few minutes; highest statistical confidence)
./scripts/run-benches.sh

# Quick run (for tight feedback loops)
./scripts/run-benches.sh --quick

# A single bench
./scripts/run-benches.sh --bench session_auth_bench
```

Results are written to:

- `bench-results/latest.json` — the current run's full manifest
- `bench-results/runs.jsonl`  — append-only time-series of every run
- `target/criterion/`         — Criterion's raw output (HTML reports, raw samples)

Open `target/criterion/report/index.html` in a browser for interactive plots.

## What each bench measures

| Bench                   | Hot path it covers                                         |
|-------------------------|-----------------------------------------------------------|
| `auth_bench`            | Sidecar token generation; provided-token passthrough      |
| `session_auth_bench`    | PASETO validate (hot/cold/revoked); EIP-191 recovery; full challenge→token roundtrip |
| `rate_limit_bench`      | `RateLimiter::check` at 1 / 100 / 1k / 10k IPs; full-window rescan |
| `circuit_breaker_bench` | `check_health` closed / open / state transitions          |
| `scoped_session_bench`  | `resolve_bearer` at 1 / 100 / 1k / 10k sessions           |
| `util_bench`            | Shell escaping; snapshot command construction (accept + reject paths); JSON parsing |
| `http_bench`            | URL construction; auth-header building                    |
| `store_bench`           | Insert / get / update / find / values at 100 / 1k / 10k records; concurrent readers during writes |
| `crypto_bench`          | HKDF-SHA256 derivation; ChaCha20-Poly1305 seal/open at 64B / 1KB / 16KB / 64KB payloads |

## Interpreting results

Each bench produces the full statistical summary:

- `mean_ns`, `median_ns`, `stddev_ns`, `variance_ns2`
- `p50`, `p90`, `p95`, `p99`, `p999`
- `mad_ns` — median absolute deviation (robust to outliers)
- `ci_lower_ns`, `ci_upper_ns` — 95% bootstrap CI for the mean
- `outliers_high`, `outliers_low` — Tukey 1.5·IQR fence
- `throughput_ops_per_sec`

When comparing runs, use CI bounds rather than point estimates. A regression
is only "real" when the current run's lower CI bound exceeds the baseline's
upper CI bound AND the relative change exceeds the threshold.

## Regression detection

`bench-harness compare` produces a markdown comparison between two runs:

```bash
cargo run -p bench-harness --release -- compare \
    --baseline bench-results/baseline.json \
    --current  bench-results/latest.json \
    --mean-threshold 0.10 \
    --p99-threshold  0.15
```

Default thresholds: **10% mean** and **15% p99**, with CI proof required.
Exits 0 on success, 1 on regression, 2 on error.

### Verdict categories

- `OK`                — within both thresholds
- `IMPROVED`          — mean dropped below -10%
- `REGRESSED (mean)`  — mean grew beyond +10% with CI proof
- `REGRESSED (p99)`   — p99 grew beyond +15%
- `REGRESSED (both)`  — both thresholds exceeded
- `noisy (under-CI)`  — large change but CI bounds overlap; not flagged

## CI integration

The `bench` job in `.github/workflows/ci.yml` runs on every PR:

1. Runs the full bench suite in `--quick` mode.
2. Aggregates into a run manifest.
3. Downloads the main-branch baseline (if present) from workflow artifacts.
4. Compares and fails the job on regressions.
5. Uploads the current run as a `bench-results-<run-id>` artifact.

On main, the latest run becomes the new baseline for future PRs.

## Methodology

- **Statistical rigor**: Criterion.rs performs bootstrap resampling (100k
  iterations), Tukey outlier detection, and Student's t-test for change
  significance. We surface Criterion's stats verbatim and also compute a
  parallel set from the raw sample vector for redundancy.
- **Reproducibility**: every run captures git SHA, branch, dirty flag, host,
  OS, arch, CPU count, rustc version, target triple, build profile, and
  relevant environment variables.
- **Host awareness**: CI runners are inherently noisier than developer
  workstations. The run manifest records the host so cross-run comparisons
  can be filtered. Do not compare CI runs to laptop runs directly.
- **No fixtures**: benchmarks operate on real in-memory data structures,
  real crypto primitives, and real disk writes (for storage benches).
  Mocks would lie about performance.

## Adding a new benchmark

1. Create `sandbox-runtime/benches/<name>_bench.rs` using the existing files
   as templates. Criterion harness is required (`criterion_main!` at the
   bottom, `harness = false` in Cargo.toml).
2. Add a `[[bench]]` entry to `sandbox-runtime/Cargo.toml`:
   ```toml
   [[bench]]
   name = "<name>_bench"
   harness = false
   ```
3. Add the bench name to the list in `scripts/run-benches.sh`.
4. Run locally with `./scripts/run-benches.sh --bench <name>_bench` to verify.

## Troubleshooting

- "No Criterion output found" — run `cargo bench` first, or check that you're
  running from the workspace root.
- Flaky CI runs — Criterion's default measurement time may be too short on
  shared runners. Increase `--measurement-time` or tighten the threshold.
- Regression on a change you believe is neutral — inspect the
  `target/criterion/<bench>/report/index.html` for the sample distribution.
  The `Noisy` verdict indicates the change couldn't be proven statistically.
