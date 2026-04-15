# Evolve / Pursue Progress

## 2026-04-15 — Generation 1: Benchmark infrastructure (COMPLETED)

**Pursuit**: `.evolve/pursuits/2026-04-15-bench-infra.md`
**Verdict**: ADVANCE

### Shipped

- `bench-harness` workspace crate with stats / manifest / compare modules
  (24 unit tests)
- 9 Criterion benchmark files covering auth, session_auth, scoped_session,
  rate_limit, circuit_breaker, util, http, store, crypto
- CI regression gate in `.github/workflows/ci.yml` (bench job)
- `docs/BENCHMARKS.md` methodology and usage docs
- `scripts/run-benches.sh` one-shot runner with --quick mode

### First baseline (partial, local run)

- auth/generate_token: 1.59 µs mean, 2.45 µs p99
- auth/require_sidecar_token: 38.7 ns mean
- scoped_session/resolve_bearer: 116 ns @ 1 session → 22.8 µs @ 10k sessions
  (196× degradation — confirms /harden diagnosis of unconditional GC)
- scoped_session/create_access_token: 139 µs mean

### Next

Hand off to `/evolve`. Primary target:
- `scoped_session::resolve_bearer` @ 10k sessions: current 22.8µs mean → goal < 5µs
- See pursuit spec "Seeds for Gen 2" for the full ranked list
