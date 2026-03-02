# Layered Architecture

This document defines the target layer model across the blueprint ecosystem and the dependency rules we enforce.

## Core Rule: Jobs vs Reads

On-chain jobs are for state-changing operations only.

Read-only access must use one of:
- `eth_call` against view/pure contract methods
- off-chain HTTP APIs
- off-chain background services (indexers, pollers, caches, metrics, schedulers)

No read-only query should be introduced as an on-chain job.

## Layers and Boundaries

### Layer 0: Infrastructure (`microvm-blueprint`)

Purpose:
- Compute substrate primitives and low-level execution plumbing
- MicroVM lifecycle foundations and host/runtime integration points
- Infra concerns needed by multiple runtimes

Must not own:
- Product workflows
- Product business policy
- User-facing feature semantics

### Layer 1: Runtime Contracts + Adapters (`sandbox-runtime`)

Purpose:
- Stable runtime-facing contracts used by products
- Adapters to infra capabilities from Layer 0
- Shared auth/session, operator API, provisioning lifecycle, metrics, and policy enforcement hooks

Must not own:
- Product-specific feature decisions
- Product-specific UX or route semantics
- Cross-product coupling

### Layer 2: Products

Products in scope:
- `ai-agent-sandbox-blueprint`
- `ai-trading-blueprints`
- `openclaw-hosting-blueprint`

Purpose:
- User-facing workflows and business logic
- Product-specific composition of runtime capabilities
- Product-specific contract/job catalogs and API endpoints

Must not own:
- Infra internals from Layer 0
- Re-implementation of shared runtime abstractions already in Layer 1
- Direct dependencies on other product repositories

## Dependency Direction Matrix

`Y` = allowed direct dependency, `N` = forbidden direct dependency.

| From \\ To | `microvm-blueprint` (L0) | `sandbox-runtime` (L1) | Products (L2) |
|---|---:|---:|---:|
| `microvm-blueprint` (L0) | N | N | N |
| `sandbox-runtime` (L1) | Y | N | N |
| Products (L2) | N | Y | N |

Interpretation:
- Flow is strictly upward in abstraction: L2 -> L1 -> L0.
- L2 -> L0 direct imports/calls are not allowed; integrate through L1 adapters.
- Product-to-product dependencies are not allowed.

## Allowed Interface Types by Layer

- L0 publishes infra primitives and host-level integration points.
- L1 publishes stable runtime contracts/adapters and service APIs consumed by products.
- L2 publishes product APIs and on-chain job definitions for product state transitions.

## Architecture Decision Guidance

When adding new behavior:
1. If it is infra-generic and reusable across runtimes, place it in L0.
2. If it is runtime-generic and reusable across products, place it in L1.
3. If it is business-feature specific, place it in L2.
4. If operation is read-only, do not add an on-chain job; use `eth_call` or off-chain read paths.

## 4-Week Migration Plan

### Week 1: Inventory and Rule Enforcement Baseline

- Produce a dependency inventory for each repo (`microvm-blueprint`, `sandbox-runtime`, and each product repo).
- Tag each module by layer ownership (`L0`, `L1`, `L2`).
- Identify all on-chain jobs currently used for reads and classify them for replacement.
- Add CI checks for forbidden dependency edges (L2 -> L0 and L2 -> L2).

Exit criteria:
- Ownership map published.
- Blocklist CI checks active in all participating repos.

### Week 2: Extract Runtime Contracts and Adapters

- Move shared runtime concerns from product repos into `sandbox-runtime`.
- Add/normalize adapter interfaces in `sandbox-runtime` for infra access currently done directly by products.
- Version and publish runtime contracts consumed by all products.

Exit criteria:
- Shared runtime interfaces consumed from `sandbox-runtime`.
- No new direct L2 -> L0 dependencies introduced.

### Week 3: Product Convergence

- Update `ai-agent-sandbox-blueprint`, `ai-trading-blueprints`, and `openclaw-hosting-blueprint` to depend only on L1 for runtime/infra needs.
- Remove duplicated runtime logic from product repos.
- Replace read-only jobs with `eth_call` and off-chain HTTP/background reads.

Exit criteria:
- All read-only flows migrated off jobs.
- Products compile and pass tests with L2 -> L1 integration only.

### Week 4: Stabilization and Governance

- Add architecture tests/linters for layer boundaries.
- Freeze contract/adaptor surfaces in `sandbox-runtime` and document versioning policy.
- Publish migration retrospective and backlog for remaining edge cases.

Exit criteria:
- Boundary checks enforced in CI.
- Layer contracts documented and ratified.
- Remaining work captured as explicit follow-up issues.
