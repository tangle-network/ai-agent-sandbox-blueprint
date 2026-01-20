# AI Agent Sandbox Blueprint - Design

## Summary

This blueprint is a sidecar-only model. Operators provide compute by running sidecar containers
locally via Docktopus (Docker). The blueprint runtime provisions containers, returns a per-sandbox
bearer token, and proxies write-only job calls to the sidecar API. No centralized orchestrator is
required or used.

## Feature Map (1:1 with Implementation)

### Sandbox Lifecycle (write-only)
- Create / stop / resume / delete sidecar containers (local Docker)
- Snapshot via sidecar `/exec` (uploads to customer-provided URL)

Jobs:
- `JOB_SANDBOX_CREATE` (0)
- `JOB_SANDBOX_STOP` (1)
- `JOB_SANDBOX_RESUME` (2)
- `JOB_SANDBOX_DELETE` (3)
- `JOB_SANDBOX_SNAPSHOT` (4)

### Sidecar Execution (write-only)
- `/exec` shell command
- `/agents/run` prompt (single turn)
- `/agents/run` task (multi-turn)

Jobs:
- `JOB_EXEC` (10)
- `JOB_PROMPT` (11)
- `JOB_TASK` (12)

### Batch Operations (operator-local)
- Create N sidecars locally
- Run task/exec across sidecar URLs
- Collect in-memory batch results

Jobs:
- `JOB_BATCH_CREATE` (20)
- `JOB_BATCH_TASK` (21)
- `JOB_BATCH_EXEC` (22)
- `JOB_BATCH_COLLECT` (23)

### Workflows (on-chain registry + cron tick)
- Store workflow configs on-chain when `JOB_WORKFLOW_CREATE` results are submitted
- Operators rebuild schedules on startup from on-chain registry
- Cron tick executes due workflows locally

Jobs:
- `JOB_WORKFLOW_CREATE` (30)
- `JOB_WORKFLOW_TRIGGER` (31)
- `JOB_WORKFLOW_CANCEL` (32)
- `JOB_WORKFLOW_TICK` (33) (internal scheduler)

### SSH Access (write-only)
- Manage authorized_keys via sidecar `/exec`

Jobs:
- `JOB_SSH_PROVISION` (40)
- `JOB_SSH_REVOKE` (41)

## Operator Selection

Operator selection is validated on-chain and can be deterministic. Clients should call
`previewOperatorSelection(count, seed)` and pass the selected operators plus the encoded
`SelectionRequest` in `requestInputs`.

```solidity
struct SelectionRequest {
    uint32 operatorCount;
    bytes32 seed;
    bool enforceDeterministic;
}
```

Batch jobs require results from all operators; other jobs accept a single result.

## Job Argument Schemas (Current)

```solidity
struct SandboxCreateRequest {
    string name;
    string image;
    string stack;
    string agent_identifier;
    string env_json;
    string metadata_json;
    bool ssh_enabled;
    string ssh_public_key;
    bool web_terminal_enabled;
    uint64 max_lifetime_seconds;
    uint64 idle_timeout_seconds;
    uint64 cpu_cores;
    uint64 memory_mb;
    uint64 disk_gb;
    string sidecar_token;           // optional: if empty, operator generates one
}

struct SandboxIdRequest {
    string sandbox_id;
}

struct SandboxSnapshotRequest {
    string sidecar_url;
    string destination;
    bool include_workspace;
    bool include_state;
    string sidecar_token;
}

struct SandboxExecRequest {
    string sidecar_url;
    string command;
    string cwd;
    string env_json;
    uint64 timeout_ms;
    string sidecar_token;
}

struct SandboxPromptRequest {
    string sidecar_url;
    string message;
    string session_id;
    string model;
    string context_json;
    uint64 timeout_ms;
    string sidecar_token;
}

struct SandboxTaskRequest {
    string sidecar_url;
    string prompt;
    string session_id;
    uint64 max_turns;
    string model;
    string context_json;
    uint64 timeout_ms;
    string sidecar_token;
}

struct BatchCreateRequest {
    uint32 count;
    SandboxCreateRequest template_request;
    address[] operators;
    string distribution;
}

struct BatchTaskRequest {
    string[] sidecar_urls;
    string[] sidecar_tokens;
    string prompt;
    string session_id;
    uint64 max_turns;
    string model;
    string context_json;
    uint64 timeout_ms;
    bool parallel;
    string aggregation;
}

struct BatchExecRequest {
    string[] sidecar_urls;
    string[] sidecar_tokens;
    string command;
    string cwd;
    string env_json;
    uint64 timeout_ms;
    bool parallel;
}

struct BatchCollectRequest {
    string batch_id;
}

struct WorkflowCreateRequest {
    string name;
    string workflow_json;
    string trigger_type;      // "manual" | "cron"
    string trigger_config;    // cron expression
    string sandbox_config_json;
}

struct WorkflowControlRequest {
    uint64 workflow_id;
}

struct SshProvisionRequest {
    string sidecar_url;
    string username;
    string public_key;
    string sidecar_token;
}

struct SshRevokeRequest {
    string sidecar_url;
    string username;
    string public_key;
    string sidecar_token;
}
```

Task spec expected in `workflow_json`:

```json
{
  "sidecar_url": "https://operator.example/sidecar",
  "prompt": "run daily report",
  "session_id": "optional",
  "max_turns": 4,
  "model": "optional",
  "context_json": "{...}",
  "timeout_ms": 60000,
  "sidecar_token": "required"
}
```

## Sidecar Auth Model

- Each sandbox gets a unique bearer token.
- Token is returned in `JOB_SANDBOX_CREATE` response.
- All sidecar jobs require the matching `sidecar_token`.

## On-Chain Workflow Registry

The blueprint contract stores workflow configs when `JOB_WORKFLOW_CREATE` results are submitted.
Operators rebuild schedules on startup by reading the registry:
- `getWorkflowIds(bool activeOnly)`
- `getWorkflow(uint64 workflowId)`

## Runtime Configuration

- `SIDECAR_IMAGE` (default: `ghcr.io/tangle-network/sidecar:latest`)
- `SIDECAR_PUBLIC_HOST` (default: `127.0.0.1`)
- `SIDECAR_HTTP_PORT` (default: `8080`)
- `SIDECAR_SSH_PORT` (default: `22`)
- `SIDECAR_PULL_IMAGE` (default: `true`)
- `DOCKER_HOST` (optional docker socket override)
- `REQUEST_TIMEOUT_SECS` (default: `30`)
- `WORKFLOW_CRON_SCHEDULE` (default: `0 * * * * *`)

## Output Model

- Job outputs are returned off-chain via the blueprint runtime.
- On-chain state is limited to workflow registry + service lifecycle.
