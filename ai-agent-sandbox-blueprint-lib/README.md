# AI Agent Sandbox Blueprint

Event-driven, multi-sandbox blueprint for the Tangle Network. Each operator manages a fleet of independent sandbox containers — customers create, use, and destroy sandboxes on demand.

## Overview

This blueprint follows an **event-driven, multi-tenant model**:

- **Multi-operator, many sandboxes**: A service has multiple operators, each hosting a fleet of sandboxes. When a sandbox is created, the contract assigns it to one operator via capacity-weighted selection. All subsequent jobs for that sandbox are routed to its assigned operator.
- **Explicit sandbox addressing**: Every job call includes `sidecar_url` and `sidecar_token` to target a specific sandbox. The customer manages these references.
- **Full lifecycle management**: Create, stop, resume, snapshot, and delete sandboxes. Multi-tier state machine (Running → Stopped → Warm image → Cold S3 → Gone).
- **Capacity-weighted operator selection**: Operators register max capacity; the contract tracks active sandbox counts and assigns new sandboxes to operators with available capacity.
- **Batch operations**: Create N sandboxes at once, run tasks/exec across multiple sandboxes in parallel.
- **Scheduled workflows**: Cron-based workflow engine for recurring agent tasks.

## Jobs (17 total)

| ID | Job | Description |
|----|-----|-------------|
| 0 | `SANDBOX_CREATE` | Create a new sandbox container |
| 1 | `SANDBOX_STOP` | Stop a running sandbox |
| 2 | `SANDBOX_RESUME` | Resume from hot/warm/cold state |
| 3 | `SANDBOX_DELETE` | Delete a sandbox and its resources |
| 4 | `SANDBOX_SNAPSHOT` | Snapshot sandbox to S3/HTTP destination |
| 10 | `EXEC` | Execute a shell command in a sandbox |
| 11 | `PROMPT` | Single-turn LLM agent interaction |
| 12 | `TASK` | Multi-turn LLM agent session |
| 20 | `BATCH_CREATE` | Create N sandboxes at once |
| 21 | `BATCH_TASK` | Run task across multiple sandboxes |
| 22 | `BATCH_EXEC` | Run command across multiple sandboxes |
| 23 | `BATCH_COLLECT` | Collect batch results |
| 30 | `WORKFLOW_CREATE` | Create a scheduled workflow |
| 31 | `WORKFLOW_TRIGGER` | Manually trigger a workflow |
| 32 | `WORKFLOW_CANCEL` | Cancel a workflow |
| 33 | `WORKFLOW_TICK` | Internal scheduler tick |
| 40 | `SSH_PROVISION` | Add SSH key to a sandbox |
| 41 | `SSH_REVOKE` | Remove SSH key from a sandbox |

## Architecture

```
Customer → Tangle (on-chain) → Blueprint Runner (Rust) → Docker → Sidecar Containers
                                     │
                                     ├── Sandbox lifecycle (create/stop/resume/delete)
                                     ├── Exec/Prompt/Task → sidecar HTTP API
                                     ├── Batch orchestration
                                     ├── Workflow scheduler (cron)
                                     ├── Reaper (idle/lifetime enforcement)
                                     └── GC (hot→warm→cold→gone tiering)
```

## Pricing (8 tiers)

| Tier | Multiplier | Jobs |
|------|-----------|------|
| 1x | Trivial | EXEC, SSH_REVOKE, SANDBOX_STOP, SANDBOX_DELETE |
| 2x | Light state | SSH_PROVISION, SANDBOX_RESUME |
| 5x | I/O-heavy | SANDBOX_SNAPSHOT |
| 10x | Container lifecycle | SANDBOX_CREATE |
| 20x | Single LLM call | PROMPT |
| 50x | Batch (small) | BATCH_CREATE, BATCH_EXEC |
| 250x | Multi-turn agent | TASK |
| 500x | Batch AI | BATCH_TASK, WORKFLOW_CREATE |

## When to use this blueprint

Choose the **Sandbox Blueprint** when you need:

- Multiple independent sandboxes per customer
- On-demand sandbox lifecycle (create/destroy)
- Batch operations across sandbox fleets
- Scheduled/recurring agent workflows
- Fine-grained per-sandbox billing
- Operator capacity management

## Comparison with Instance Blueprint

| Feature | Sandbox Blueprint | Instance Blueprint |
|---------|------------------|-------------------|
| Model | Event-driven, multi-tenant | Subscription-based, 1:1 |
| Sandboxes per operator | Many (fleet) | One (singleton) |
| Addressing | Explicit `sidecar_url` + `sidecar_token` | Implicit (operator auto-resolves) |
| Multi-operator | Yes — each sandbox assigned to 1 operator via capacity-weighted selection | Yes — all N operators run identical copies, respond independently |
| Operator selection | Capacity-weighted assignment at sandbox creation | Customer chooses N operators at service creation |
| Result aggregation | One operator responds per sandbox | Prompt/task: all N operators respond, contract stores per-operator result hashes |
| Batch/Workflow | Yes | No |
| On-chain state | Sandbox registry, capacity, workflows | Operator endpoints, result hashes, TEE attestations |
| Best for | Platforms, dev tools, CI/CD | Dedicated agents, TEE verification, consensus |

## Smart Contract

`AgentSandboxBlueprint.sol` — tracks sandbox→operator mapping, operator capacity, deterministic operator selection, and on-chain workflow registry.

## Testing

```bash
# Unit + wiremock integration tests
cargo test -p ai-agent-sandbox-blueprint-lib

# Real sidecar tests (requires Docker)
REAL_SIDECAR=1 cargo test -p ai-agent-sandbox-blueprint-lib

# Snapshot tests (requires Docker + MinIO)
docker compose -f docker-compose.test.yml up -d
SNAPSHOT_TEST=1 cargo test -p ai-agent-sandbox-blueprint-lib
```
