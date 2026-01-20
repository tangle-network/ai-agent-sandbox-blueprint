# AI Agent Sandbox Blueprint - Architecture Design

## Overview

This document defines the architecture for the AI Agent Sandbox Blueprint, a decentralized control plane for AI agent execution infrastructure. The blueprint enables third-party operators to provide compute resources (sandboxes, sidecars) on the Tangle network, with protocol-native payments and smart contract-based orchestration.

## Architecture Model

### Two Execution Paths

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Customer/Developer                              │
└─────────────────────────────────────────────────────────────────────────┘
                    │                              │
                    ▼                              ▼
┌───────────────────────────────┐  ┌───────────────────────────────────────┐
│     CENTRALIZED PATH          │  │         DECENTRALIZED PATH            │
│                               │  │                                       │
│  SDK Client                   │  │  SDK Client                           │
│      │                        │  │      │                                │
│      ▼                        │  │      ▼                                │
│  Orchestrator API             │  │  Blueprint Contract (on-chain)        │
│      │                        │  │      │                                │
│      ▼                        │  │      │ requestService()               │
│  Container Driver             │  │      │ callJob()                      │
│      │                        │  │      ▼                                │
│      ▼                        │  │  Operator(s) running Blueprint        │
│  Sidecar(s)                   │  │      │                                │
│                               │  │      ▼                                │
│                               │  │  Sidecar(s)                           │
└───────────────────────────────┘  └───────────────────────────────────────┘
```

### Blueprint as Decentralized Orchestrator

The blueprint smart contract performs the same coordination functions as the centralized orchestrator:

| Function | Centralized (Orchestrator) | Decentralized (Blueprint) |
|----------|---------------------------|---------------------------|
| Operator selection | Internal load balancer | Smart contract routes to registered operators |
| Payment | Stripe/credit system | Protocol-native payments |
| Batch coordination | Orchestrator fans out | Contract coordinates across operators |
| Scheduling | Internal scheduler | CronJob producer in operator runtime |
| State management | Redis/DB | On-chain routing + operator local state |

### Operator Selection and Fan-Out

Operator selection is validated on-chain and driven by a deterministic selection helper.
Clients should call `previewOperatorSelection(count, seed)` before requesting a service and
pass both the operator list and the encoded selection request in `requestInputs`.

Selection request encoding:

```solidity
struct SelectionRequest {
    uint32 operatorCount;
    bytes32 seed;
    bool enforceDeterministic;
}
```

If `enforceDeterministic` is true, the contract verifies the operator list matches the
deterministic selection over eligible operators for this blueprint. Fan-out is achieved by
creating services with N operators; batch jobs require results from all operators.

## Job Surface

### Complete Job IDs

```rust
// ═══════════════════════════════════════════════════════════════════════
// SANDBOX LIFECYCLE (state-changing)
// ═══════════════════════════════════════════════════════════════════════

pub const JOB_SANDBOX_CREATE: u8 = 0;      // Provision sidecar + optional SSH access
pub const JOB_SANDBOX_STOP: u8 = 1;        // Pause (keeps state)
pub const JOB_SANDBOX_RESUME: u8 = 2;      // Resume from pause
pub const JOB_SANDBOX_DELETE: u8 = 3;      // Terminate + cleanup
pub const JOB_SANDBOX_SNAPSHOT: u8 = 4;    // Write state to customer storage

// ═══════════════════════════════════════════════════════════════════════
// EXECUTION (state-changing, billable)
// ═══════════════════════════════════════════════════════════════════════

pub const JOB_EXEC: u8 = 10;               // Run shell command
pub const JOB_PROMPT: u8 = 11;             // Single agent turn
pub const JOB_TASK: u8 = 12;               // Full task (multi-turn until done)

// ═══════════════════════════════════════════════════════════════════════
// BATCH OPERATIONS (state-changing, fan-out across operators)
// ═══════════════════════════════════════════════════════════════════════

pub const JOB_BATCH_CREATE: u8 = 20;       // Create N sandboxes across operators
pub const JOB_BATCH_TASK: u8 = 21;         // Run task on multiple sandboxes
pub const JOB_BATCH_EXEC: u8 = 22;         // Run command on multiple sandboxes
pub const JOB_BATCH_COLLECT: u8 = 23;      // Gather results from batch

// ═══════════════════════════════════════════════════════════════════════
// WORKFLOWS (scheduled/triggered via CronJob producer)
// ═══════════════════════════════════════════════════════════════════════

pub const JOB_WORKFLOW_CREATE: u8 = 30;    // Define workflow (tasks + triggers)
pub const JOB_WORKFLOW_TRIGGER: u8 = 31;   // Manual trigger
pub const JOB_WORKFLOW_CANCEL: u8 = 32;    // Cancel running workflow

// ═══════════════════════════════════════════════════════════════════════
// SSH/ACCESS (state-changing)
// ═══════════════════════════════════════════════════════════════════════

pub const JOB_SSH_PROVISION: u8 = 40;      // Setup SSH access → credentials
pub const JOB_SSH_REVOKE: u8 = 41;         // Revoke SSH access
```

### Request/Response Types

Job responses are delivered off-chain to the caller via the blueprint runtime.
No job outputs are stored on-chain. On-chain state is limited to service lifecycle
and operator assignment handled by the protocol.

```solidity
// ═══════════════════════════════════════════════════════════════════
// SANDBOX LIFECYCLE
// ═══════════════════════════════════════════════════════════════════

struct SandboxCreateRequest {
    // Resources
    uint64 cpu_cores;
    uint64 memory_mb;
    uint64 disk_gb;

    // Configuration
    string agent_backend;           // "opencode", "claude-agent", etc.
    string agent_identifier;        // Agent profile ID
    string env_json;                // Environment variables (JSON)

    // Access
    bool ssh_enabled;
    string ssh_public_key;          // Customer's public key

    // Lifecycle
    uint64 ttl_blocks;              // Duration in blocks
    uint64 idle_timeout_seconds;    // Auto-stop if idle

    // Customer storage for snapshots
    string snapshot_destination;    // s3://bucket/path or ipfs://...
}

struct SandboxCreateResponse {
    string sandbox_id;
    string sidecar_endpoint;        // Direct HTTP endpoint
    string ssh_host;                // SSH hostname
    uint16 ssh_port;                // SSH port
    string ssh_user;                // SSH username
    string stream_endpoint;         // SSE stream URL for events
    uint64 expires_at_block;        // When sandbox expires
}

// ═══════════════════════════════════════════════════════════════════
// TASK EXECUTION
// ═══════════════════════════════════════════════════════════════════

struct TaskRequest {
    string sidecar_endpoint;        // Direct HTTP endpoint
    string prompt;                  // What to do
    string session_id;              // Optional: continue session
    uint64 max_turns;               // Max agent turns (0 = unlimited)
    uint64 timeout_ms;              // Overall timeout
    string context_json;            // Additional context
    string model;                   // Optional model override
}

struct TaskResponse {
    bool success;
    string result;                  // Final response
    string error;                   // Error if failed
    string trace_id;                // For debugging/telemetry
    uint32 turns_used;              // Agent turns consumed
    uint64 duration_ms;
    uint32 input_tokens;
    uint32 output_tokens;
    string session_id;              // For continuation
}

// ═══════════════════════════════════════════════════════════════════
// BATCH OPERATIONS
// ═══════════════════════════════════════════════════════════════════

struct BatchCreateRequest {
    uint32 count;                   // How many sandboxes
    SandboxCreateRequest template;  // Config template for each
    address[] operators;            // Which operators (empty = auto-select)
    string distribution;            // "round_robin" | "cheapest" | "random"
}

struct BatchCreateResponse {
    string batch_id;
    string[] sandbox_ids;
    string[] endpoints;
}

struct BatchTaskRequest {
    string batch_id;                // Optional batch_id for tracking
    string[] sidecar_endpoints;     // Explicit sidecar endpoints
    string prompt;                  // Same task for all
    bool parallel;                  // Run in parallel or sequential
    string aggregation;             // "all" | "first_success" | "majority"
}

struct BatchTaskResponse {
    string batch_id;
    TaskResponse[] results;
    uint32 succeeded;
    uint32 failed;
}

// ═══════════════════════════════════════════════════════════════════
// WORKFLOWS
// ═══════════════════════════════════════════════════════════════════

struct WorkflowCreateRequest {
    string name;
    string workflow_json;           // Workflow definition (JSON task spec for now)
    string trigger_type;            // "manual" | "cron" | "webhook" | "event"
    string trigger_config;          // Cron expression or webhook config
    string sandbox_config_json;     // SandboxCreateRequest as JSON
}

struct WorkflowCreateResponse {
    uint64 workflow_id;
    string status;                  // "active" | "paused"
}

// workflow_id is the JobCall ID emitted by the protocol for the create request.
// workflow_json should include sidecar_url + prompt fields if you expect the
// blueprint to execute the task directly.

struct WorkflowControlRequest {
    uint64 workflow_id;
}

// ═══════════════════════════════════════════════════════════════════
// SNAPSHOT
// ═══════════════════════════════════════════════════════════════════

struct SandboxSnapshotRequest {
    string sidecar_endpoint;
    string destination;             // Customer-provided storage URI
    bool include_workspace;         // Include /workspace directory
    bool include_state;             // Include sidecar state
}

struct SandboxSnapshotResponse {
    bool success;
    string snapshot_uri;            // Where it was written
    uint64 size_bytes;
}
```

## Customer Journey

```
1. Customer queries available operators (off-chain indexer or chain read)
   → Gets list: [operator_A, operator_B, operator_C] with pricing

2. Customer calls requestService() on blueprint contract:
   - blueprintId: AI_SANDBOX_BLUEPRINT
   - operators: [operator_A]  (or empty for auto-selection)
   - ttl: 3600 blocks (~6 hours)
   - paymentAmount: calculated via protocol pricing

3. Operator receives onRequest() hook
   → Provisions sidecar container
   → Returns endpoint + SSH credentials off-chain to the caller

4. Customer uses sandbox:
   - HTTP API: prompt, exec, task
   - SSE stream to their apps
   - SSH for interactive access

5. Customer snapshots data before TTL (their responsibility):
   - JOB_SANDBOX_SNAPSHOT → writes to customer-provided S3/IPFS
   - Or customer pulls files via SSH/API

6. TTL expires or customer calls JOB_SANDBOX_DELETE
   → Operator tears down resources
   → Final settlement on-chain
```

## Operator Requirements

### Minimal Operator Stack

```
┌─────────────────────────────────────────┐
│         Operator Host                   │
├─────────────────────────────────────────┤
│  ┌─────────────────────────────┐       │
│  │   Blueprint Runtime          │       │
│  │   (this crate)               │       │
│  │   - Receives job calls       │       │
│  │   - Provisions containers    │       │
│  │   - Returns results          │       │
│  └─────────────────────────────┘       │
│                │                        │
│                ▼                        │
│  ┌─────────────────────────────┐       │
│  │   Sidecar Container(s)       │       │
│  │   - AGENT_BACKEND=opencode   │       │
│  │   - Customer or operator     │       │
│  │     LLM API keys             │       │
│  └─────────────────────────────┘       │
│                                         │
│  ┌─────────────────────────────┐       │
│  │   SSH Gateway (optional)     │       │
│  │   - Tunnels to sidecars      │       │
│  └─────────────────────────────┘       │
└─────────────────────────────────────────┘
```

### Not Required for Operators

Operators do NOT need the full orchestrator stack:
- No multi-tenant session management (blueprint handles routing)
- No complex autoscaling (operator scales their own fleet)
- No storage snapshots (customer's responsibility)
- No credit/billing system (payments handled by protocol)

### Sidecar Standalone Mode

Sidecars can run independently without orchestrator:

```bash
docker run -p 8080:8080 \
  -e SIDECAR_PORT=8080 \
  -e AGENT_WORKSPACE_ROOT=/workspace \
  -e AGENT_BACKEND=opencode \
  -e OPENCODE_MODEL_API_KEY=sk-... \
  sidecar:latest
```

Required environment:
- `SIDECAR_PORT` - HTTP port (default: 8080)
- `AGENT_WORKSPACE_ROOT` - Workspace directory
- `AGENT_BACKEND` - AI backend ("opencode", "claude-agent", etc.)
- `OPENCODE_MODEL_API_KEY` - LLM API key (or customer provides per-request)

Optional:
- `SIDECAR_AUTH_TOKEN` - Simple bearer token auth
- `STORAGE_PATH` - Persistent state location

## SSH Access Model

```
┌─────────────┐                    ┌─────────────┐
│  Customer   │                    │  Operator   │
│             │                    │             │
│  ssh -i key │ ───────────────────│─┐           │
│  user@host  │    SSH tunnel      │ │ SSH GW    │
│             │                    │ │           │
└─────────────┘                    │ └──────┐    │
                                   │        ▼    │
                                   │ ┌──────────┐│
                                   │ │ Sidecar  ││
                                   │ │ Container││
                                   │ └──────────┘│
                                   └─────────────┘
```

SSH credentials returned in `SandboxCreateResponse`:
- `ssh_host`: Operator's SSH gateway hostname
- `ssh_port`: Allocated port for this sandbox
- `ssh_user`: Generated username (usually sandbox_id)

Customer's public key is injected at provision time from `SandboxCreateRequest.ssh_public_key`.

## Workflow/Cron Support (On-Chain Registry)

Workflow definitions are persisted on-chain by the blueprint contract when
`JOB_WORKFLOW_CREATE` results are submitted. Operators rebuild the schedule
on startup by reading the contract registry, so active workflows survive restarts.

The blueprint SDK's `CronJob` producer enables scheduled execution on the operator:

```rust
use blueprint_producers_extra::cron::CronJob;

async fn main() {
    let router = Router::new()
        .route(JOB_WORKFLOW_TICK, workflow_tick);

    // Check for due workflows every minute
    let workflow_cron = CronJob::new(JOB_WORKFLOW_TICK, "0 * * * * *").await?;

    blueprint_sdk::run(router, workflow_cron).await;
}

#[debug_job]
async fn workflow_tick() -> Result<JsonResponse, String> {
    let due_workflows = get_due_workflows().await;
    for workflow in due_workflows {
        execute_workflow(&workflow).await?;
    }
    Ok(JsonResponse { json: "{}".into() })
}
```

Operators can override the tick schedule with `WORKFLOW_CRON_SCHEDULE`.

## Implementation Status

| Component | Status | Notes |
|-----------|--------|-------|
| Sandbox lifecycle jobs | ✅ Complete | Includes snapshot upload |
| Exec/prompt/task jobs | ✅ Complete | Task is multi-turn agent run |
| Batch jobs | ✅ Complete | In-memory aggregation + collect |
| Workflow jobs | ✅ On-chain | Registry + cron tick runner |
| SSH provision | ✅ Complete | Authorized key management via /exec |

## Next Steps

1. Add durable workflow storage (sled/sqlite) if operators need persistence beyond chain registry
2. Extend workflow_json schema for multi-step DAGs and parallel steps
3. Add IPFS snapshot support if operators need decentralized storage
