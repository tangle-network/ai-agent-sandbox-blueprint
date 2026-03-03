# Blueprint Factory Status

Date: March 3, 2026

This document tracks execution reality across the layered blueprint ecosystem, not just target architecture.

## Current Snapshot

| Repo | Status | What is already strong | Blocking gaps for fast new blueprint delivery |
|---|---|---|---|
| `ai-agent-sandbox-blueprint` | Active | Direct-report instance lifecycle architecture is in place; cloud/instance/tee modes are aligned to 5 on-chain jobs; tests are green in UI, Solidity, and Rust paths | Product code still owns significant runtime coupling; layer contracts in docs are not yet fully enforced in code |
| `microvm-blueprint` | Green tests | Clean provider boundary (`VmProvider`/`VmQuery`), lifecycle job wiring, query service | Still in-memory adapter only; no Firecracker-backed provider, no durable VM state, no production lifecycle supervision |
| `openclaw-sandbox-blueprint` | Green tests | Product lifecycle handlers, ownership checks, and stable ABI/job structure | Lifecycle currently mutates local state directly; runtime adapter boundary is documented but not wired; query API and runtime delegation are still pending |
| `blueprint-ui` + product UIs | Reusable primitives in place | Shared chain/hooks/primitives are widely consumed; layout primitives are being consolidated | High-value feature flows are still app-local (`InfrastructureModal`, resource detail tab shells, and some orchestration UX), slowing new app spin-up |

## Highest-ROI Execution Order

### P0 (completed in this branch): Restart-safe direct lifecycle reconciliation

- `ai-agent-instance-blueprint-lib` now reconciles local provisioned state with on-chain `isOperatorProvisioned` on startup.
- If local state exists but chain state is missing, operator self-reports `reportProvisioned` from persisted record.
- This removes a permanent drift class caused by transient report failures after local provisioning.

### P1: Enforce runtime adapter contracts in code (not only docs)

1. Introduce concrete trait-backed adapter surfaces in `sandbox-runtime` for lifecycle + query paths.
2. Move direct runtime calls in product crates behind those adapters.
3. Add compile-time guardrails (forbidden imports) for `L2 -> L0` edges.

Definition of done:
- Product crates depend on adapter contracts only.
- CI fails on forbidden layer imports.

### P2: Ship Firecracker-capable provider in `microvm-blueprint`

1. Add `FirecrackerVmProvider` implementing `VmProvider` + `VmQuery`.
2. Add durable VM metadata store and process supervision/recovery.
3. Add conformance tests that run against both in-memory and Firecracker providers.

Definition of done:
- Same lifecycle test suite passes for in-memory and Firecracker providers.
- Crash/restart recovery retains authoritative VM state.

### P3: Migrate OpenClaw lifecycle to runtime adapter delegation

1. Replace direct local state mutations in lifecycle handlers with adapter calls.
2. Keep local store as projection/cache, not authority.
3. Add operator query API surface for list/detail/health.

Definition of done:
- OpenClaw lifecycle logic is provider-agnostic.
- Runtime backend can be swapped without changing job handlers.

### P4: Maximize UI reuse for blueprint creation speed

1. Promote duplicated flow components into shared packages:
   - infra selection modal/bar
   - resource detail tabs/shell
   - standard confirmation + progress flows
2. Keep app-local code only for blueprint-specific semantics.
3. Publish a small "new blueprint UI starter" composed from shared primitives.

Definition of done:
- New blueprint UI can be launched from shared components with minimal app-local code.
- Cross-app drift is prevented by package-level component ownership.

## Naming and Boundary Cleanup (recommended)

- Standardize on `openclaw-hosting-blueprint` naming everywhere (repo, docs, references) to avoid split terminology with `openclaw-sandbox-blueprint`.
- Keep `microvm-blueprint` as the infrastructure identity and avoid product-layer imports from it.
- Keep instance lifecycle terminology strictly "direct report" (`reportProvisioned`/`reportDeprovisioned`) across code and docs.
