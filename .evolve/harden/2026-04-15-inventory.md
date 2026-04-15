# Harden Inventory — ai-agent-sandbox-blueprint
Date: 2026-04-15

## Test infra
- **Runner (Rust)**: `cargo test` across 7 workspace crates
- **Runner (UI)**: vitest + playwright (e2e)
- **Locations**:
  - `sandbox-runtime/tests/` (firecracker, ssh_e2e, tee_integration)
  - `ai-agent-sandbox-blueprint-lib/tests/` (anvil, e2e_operator_api, integration, operator_api_integration, real_sidecar, sidecar_integration, snapshot_integration)
  - `ai-agent-instance-blueprint-lib/tests/` (billing_anvil, billing_lifecycle, e2e_instance, integration, real_sidecar)
  - `ai-agent-tee-instance-blueprint-lib/tests/` (tee_config, tee_integration, tee_provision, unit)
  - `ui/src/test/`, `ui/src/tests/`, `ui/e2e/`
- **Real-vs-mocked ratio**: Good — multiple tiers (unit with wiremock, real sidecar with REAL_SIDECAR=1, e2e with SIDECAR_E2E=1, snapshot with SNAPSHOT_TEST=1)
- **Coverage**: No coverage reports configured

## Eval infra
- **None**. No `.evolve/` directory, no eval suite. Flag for /pursue.

## Benchmark infra
- **None**. No `.bench` files, no benchmark scripts in CI. Flag for /pursue.

## Observability
- No SDK-level observability detected (no OpenTelemetry, Sentry, Langfuse)
- Internal metrics: `AtomicU64` counters in `sandbox-runtime/src/metrics.rs`
- Tracing: `tracing` crate used throughout

## CI workflows
- `ci.yml` — main CI (tests, clippy, typecheck)
- `foundry.yml` — Solidity contract CI
- `publish-agent-ui.yml` — UI package publishing

## Architecture (security-critical modules)
- `auth.rs` — token generation (32 bytes, OsRng)
- `session_auth.rs` — EIP-191 challenge/response + PASETO v4.local session tokens
- `scoped_session_auth.rs` — per-resource scoped sessions
- `rate_limit.rs` — sliding-window per-IP (10/30/120/2400 req/min tiers)
- `operator_api.rs` — 50+ REST endpoints (Axum), CORS, security headers
- `secret_provisioning.rs` — 2-phase secret injection (container recreation)
- `util.rs` — snapshot SSRF prevention, shell escaping
- `circuit_breaker.rs` — per-sandbox health tracking
- `store.rs` — persistent JSON store (RwLock, no flock)
- `ssh_validation.rs` — username/key validation
- `ingress_access_control.rs` — UI bearer credential generation
- `tee/` — TEE backends + sealed secrets API
