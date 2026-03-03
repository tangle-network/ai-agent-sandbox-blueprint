# Layered Architecture

This document defines the layered blueprint model and import boundaries for:

- `microvm-blueprint` (infrastructure)
- `sandbox-runtime` (runtime contracts + adapters)
- `ai-agent-sandbox-blueprint` (product)
- `ai-trading-blueprints` (product)
- `openclaw-hosting-blueprint` (product)

## Hard Rule: Jobs Are State-Changing Only

On-chain jobs must mutate authoritative state.

- Reads must use `eth_call`.
- Operational read/write I/O (exec/prompt/task/ssh/snapshot/proxy) must use the operator HTTP API.

## Repo Roles and Boundaries

## Layer 0: `microvm-blueprint` (Infra Only)

Owns:
- Firecracker/microVM lifecycle primitives
- Host-level networking/storage/attestation substrate
- Provider-level failure and capability model

Must not own:
- Product workflows
- Product routing semantics
- Product billing/policy decisions

## Layer 1: `sandbox-runtime` (Runtime Contracts + Adapters)

Owns:
- Stable runtime interfaces consumed by products
- Adapter layer to L0 providers
- Shared auth/session/rate-limit/metrics/provision tracking

Must not own:
- Product-specific UX semantics
- Product-specific business logic
- Cross-product coupling

## Layer 2: Product Blueprints

Owns:
- Product workflow and business policy
- Product service composition
- Product-facing contract/job catalogs

Must not own:
- Direct L0 imports
- Runtime re-implementations already in L1
- Product-to-product imports

## Dependency Direction Rules

Allowed:
- `L2 -> L1`
- `L1 -> L0`

Forbidden:
- `L2 -> L0`
- `L2 -> L2`
- `L1 -> L2`

Temporary exception (current repo state):
- `ai-agent-tee-instance-blueprint-lib -> ai-agent-instance-blueprint-lib` exists as same-product variant reuse.
- Exit plan: move shared instance runtime logic to L1 and remove this edge.

## Why `microvm-blueprint` Exists Separately

Even if `sandbox-runtime` has a microVM adapter, `microvm-blueprint` stays separate because:

1. Infra lifecycle evolves at a different cadence than product/runtime APIs.
2. Multiple runtimes can share the same low-level provider substrate.
3. Firecracker/security-sensitive code requires tighter isolation and review ownership.
4. Runtime contracts can remain stable while provider internals change.

## Current State (March 3, 2026)

- `sandbox-runtime` still contains concrete Docker/TEE integrations directly.
- Runtime selection contract is wired (`metadata_json.runtime_backend`), but Firecracker provider execution is not yet implemented in this repo.
- L1 layer contracts are codified in `sandbox-runtime/src/contracts.rs`
  (`SandboxProvider`, `RuntimeAdapter`, `DefaultRuntimeAdapter`, `DockerSandboxProvider`).
- L0 is a target boundary, not fully extracted in this repo.
- Layer docs are normative for new code; existing drift should be reduced incrementally.

## Migration Phases

## Phase 0: Now

- Keep unified 5-job protocol and operator API split.
- Use direct instance lifecycle reporting (`reportProvisioned` / `reportDeprovisioned`) as canonical startup sync path.
- Enforce no new `L2 -> L0` imports.
- Document temporary exceptions explicitly in PRs.

Exit criteria:
- No newly introduced forbidden edges.
- Contract and README docs align with current runtime behavior.

## Phase 1: Next

- Extract provider contracts and concrete microVM/firecracker utilities into L0.
- Keep `sandbox-runtime` focused on adapter and runtime contract surfaces.
- Move shared instance logic used by instance + tee-instance into L1.

Exit criteria:
- No `ai-agent-tee-instance-blueprint-lib -> ai-agent-instance-blueprint-lib` edge.
- Provider-facing code consumed only through L1 adapters.

## Phase 2: Deprecation

- Deprecate direct provider/internal types leaking through product crates.
- Add CI guardrails for forbidden imports and layer checks.
- Publish versioned L1 contract compatibility policy.

Exit criteria:
- Layer boundaries validated in CI.
- Deprecated edges removed and tracked as closed migration items.
