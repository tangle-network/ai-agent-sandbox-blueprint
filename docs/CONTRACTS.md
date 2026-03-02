# Layer Contracts

This document defines interface contracts between infrastructure, runtime, and product layers.

## Non-Negotiable Contract

Jobs are state-changing only.

Read-only behavior must be implemented via:
- `eth_call` for on-chain reads
- off-chain HTTP APIs for operational reads
- off-chain background services for indexed/aggregated reads

State reads must not be encoded as on-chain jobs.

## Contract 1: Infrastructure to Runtime (`microvm-blueprint` -> `sandbox-runtime`)

Provider (`microvm-blueprint`) guarantees:
- Stable infra primitives for execution/provisioning lifecycle
- Deterministic, documented failure modes
- Versioned capability surfaces

Consumer (`sandbox-runtime`) guarantees:
- Uses infra primitives through adapter boundaries, not product policy logic
- Translates infra errors into runtime-domain errors
- Does not leak infra-only types into product APIs unless explicitly versioned

Compatibility policy:
- Breaking changes require version bump and migration notes.
- Deprecated interfaces must have a defined removal window.

## Contract 2: Runtime to Products (`sandbox-runtime` -> product repos)

Provider (`sandbox-runtime`) guarantees:
- Stable runtime contracts for auth, sessioning, provisioning, operator APIs, and lifecycle hooks
- Backward-compatible adapter behavior within a major version
- Clear semantics for state transitions exposed to products

Consumers (`ai-agent-sandbox-blueprint`, `ai-trading-blueprints`, `openclaw-hosting-blueprint`) guarantee:
- Depend on runtime contracts instead of directly binding to infra internals
- Keep business/workflow logic in product layer only
- Avoid cross-product imports and shared hidden coupling

Compatibility policy:
- Runtime contract changes require changelog entries and upgrade notes.
- Products must pin compatible runtime versions and upgrade intentionally.

## Contract 3: Product Layer

Each product guarantees:
- Product-owned job catalog only for state transitions
- Product-owned read APIs implemented as `eth_call` and/or off-chain read services
- No direct dependency on other product repos

Forbidden patterns:
- Read-only on-chain jobs
- Direct L2 -> L0 dependency
- Product-to-product dependency edges

## Dependency Rules (Enforced)

- Allowed: L2 -> L1 -> L0
- Forbidden: L2 -> L0
- Forbidden: L2 -> L2
- Forbidden: L1 -> L2

## API and Job Design Checklist

Before adding a new operation:
1. Does this operation change authoritative state?
2. If no, implement as `eth_call` or off-chain read service, not a job.
3. Which layer owns this behavior (L0/L1/L2)?
4. Are we introducing a forbidden dependency edge?
5. Have versioning and migration notes been updated if interface changes?

## Governance

- Architecture PRs that cross layer boundaries must include:
  - dependency impact
  - compatibility impact
  - migration/rollback plan
- Boundary violations should fail CI.
- Exceptions require time-boxed waivers and explicit issue tracking.
