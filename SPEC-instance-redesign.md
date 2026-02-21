# Sandbox Blueprint Redesign Spec

## Problem

Three separate Solidity contracts (`AgentSandboxBlueprint`, `AgentInstanceBlueprint`, `AgentTeeInstanceBlueprint`) with heavy duplication. Read-only operations (exec, prompt, task) are registered as on-chain jobs — they cost gas, require Tangle round-trips, and modify no contract state. Instance blueprints require a separate `JOB_PROVISION` after service creation, which is unnecessary ceremony.

## Principles

1. **On-chain jobs are for state changes only.** If a job doesn't modify contract state, it's an operator API endpoint.
2. **One contract, deployed N times.** Use flags (`instanceMode`, `teeMode`) instead of separate contracts.
3. **Service creation = deployment** for instance/TEE. No separate provision step.
4. **Operator API for everything else.** Auth via PASETO session tokens. Reads, command execution, prompt/task interactions — all HTTP.

---

## Contract Consolidation

### Before: 3 contracts

| Contract | Jobs | Unique State |
|----------|------|-------------|
| AgentSandboxBlueprint | 17 | sandboxOperator, sandboxActive, workflows, capacity tracking |
| AgentInstanceBlueprint | 8 | operatorProvisioned, operatorSidecarUrl, jobResultHash |
| AgentTeeInstanceBlueprint | 8 | Same as Instance + mandatory attestation check |

### After: 1 contract (`AgentSandboxBlueprint.sol`)

```solidity
bool public instanceMode;    // false = cloud/fleet, true = instance
bool public teeRequired;     // false = standard, true = TEE attestation enforced

function setInstanceMode(bool _mode) external onlyFromTangle { instanceMode = _mode; }
function setTeeRequired(bool _required) external onlyFromTangle { teeRequired = _required; }
```

All state variables from all 3 contracts exist in the unified contract. The flags gate which code paths execute:

- `instanceMode=false`: Cloud behavior (capacity-weighted selection, sandbox registry, workflows, batches)
- `instanceMode=true`: Instance behavior (per-operator provisioning, result hash storage, auto-vault at service init)
- `teeRequired=true`: Attestation enforcement in `_handleProvisionResult`

---

## Job Redesign

### What stays on-chain (state-changing)

These jobs modify contract storage, emit events needed for indexing, or require payment routing:

| ID | Job | Why on-chain |
|----|-----|-------------|
| 0 | `sandbox_create` | Modifies sandbox registry + operator load counters. Needs capacity-weighted routing. |
| 1 | `sandbox_delete` | Removes from registry, decrements counters. |
| 2 | `workflow_create` | Stores workflow config, timestamps, active flag. |
| 3 | `workflow_trigger` | Updates `last_triggered_at`. Triggers paid execution. |
| 4 | `workflow_cancel` | Sets `active = false`. |
| 5 | `provision` | Instance mode only. Registers operator, stores sidecar URL, increments count. |
| 6 | `deprovision` | Instance mode only. Removes operator from service. |

**7 jobs total.** Down from 17 (cloud) / 8 (instance).

### What moves to operator API (read-only or off-chain state)

| Old Job | Reason | New Endpoint |
|---------|--------|-------------|
| `exec` | Read-only. Calls sidecar, returns stdout/stderr. | `POST /api/sandboxes/{id}/exec` |
| `prompt` | Read-only. Calls sidecar AI, returns response. | `POST /api/sandboxes/{id}/prompt` |
| `task` | Read-only. Calls sidecar AI with multi-turn. | `POST /api/sandboxes/{id}/task` |
| `sandbox_stop` | No on-chain state change. `sandboxActive` only set by create/delete. | `POST /api/sandboxes/{id}/stop` |
| `sandbox_resume` | No on-chain state change. | `POST /api/sandboxes/{id}/resume` |
| `sandbox_snapshot` | No on-chain state change. | `POST /api/sandboxes/{id}/snapshot` |
| `ssh_provision` | Off-chain key provisioning. No contract state. | `POST /api/sandboxes/{id}/ssh` |
| `ssh_revoke` | Off-chain key removal. No contract state. | `DELETE /api/sandboxes/{id}/ssh` |
| `batch_create` | Returns 0 required results already. Move to API. | `POST /api/sandboxes/batch` |
| `batch_task` | Read-only. | `POST /api/sandboxes/batch/task` |
| `batch_exec` | Read-only. | `POST /api/sandboxes/batch/exec` |
| `batch_collect` | Read-only. | `GET /api/sandboxes/batch/{id}` |

All operator API endpoints require `SessionAuth` (PASETO). The auth flow already exists in `sandbox-runtime/src/operator_api.rs`.

### Instance mode: singleton endpoints

When `instanceMode=true`, operators serve one sandbox per service. The API simplifies:

| Endpoint | Notes |
|----------|-------|
| `POST /api/sandbox/exec` | No `{id}` — singleton lookup |
| `POST /api/sandbox/prompt` | No `{id}` — singleton lookup |
| `POST /api/sandbox/task` | No `{id}` — singleton lookup |
| `POST /api/sandbox/stop` | Singleton |
| `POST /api/sandbox/resume` | Singleton |
| `POST /api/sandbox/snapshot` | Singleton |
| `POST /api/sandbox/ssh` | Singleton |
| `DELETE /api/sandbox/ssh` | Singleton |

---

## Instance Auto-Provision

Same pattern as trading blueprint:

### On-chain (`onServiceInitialized`)

When `instanceMode=true`:
1. Read `_serviceConfigs[serviceId]` for asset token + signers
2. If asset token is set and signers exist (or operators as fallback): create vault via `VaultFactory`
3. Store in `instanceVault[serviceId]` and `botVaults[serviceId][0]`
4. Grant `VAULT_OPERATOR_ROLE` to all operators
5. Emit `BotVaultDeployed(serviceId, 0, vault, share)`

If no vault factory or no asset token: skip gracefully, emit `BotVaultSkipped`.

### Off-chain (operator API)

After service init, each operator's binary detects the new service and auto-provisions its sidecar:
- Binary calls `provision_core()` internally (no on-chain job needed)
- Stores singleton sandbox via `set_instance_sandbox()`
- Reports sidecar URL to contract via `onJobResult(JOB_PROVISION)` — this IS still on-chain because the contract needs the sidecar URL and attestation hash

**Wait — this contradicts "no separate provision step".** Here's the nuance:

For instance mode, `JOB_PROVISION` (job 5) is kept on-chain because the contract needs to:
- Store `operatorProvisioned[serviceId][operator] = true`
- Store `operatorSidecarUrl[serviceId][operator]`
- Store `operatorAttestationHash` (TEE)
- Emit `OperatorProvisioned` event for indexing

The difference from current: the **binary auto-submits** this job on startup (not the user). The frontend creates the service, and each operator binary self-provisions and reports back.

### Frontend flow (instance)

```
User selects instance blueprint
  → "Create Service" (single button, no separate provision step)
  → Service created on-chain
  → onServiceInitialized creates vault (if applicable)
  → Operator binaries detect new service, auto-provision sidecars
  → Each binary submits JOB_PROVISION result with sidecar URL
  → Contract stores operator info
  → Frontend polls /api/sandbox/status until ready
  → User configures secrets via /api/sandbox/secrets
  → Ready
```

---

## Multi-Operator Result Verification

Current instance contract stores `jobResultHash[serviceId][jobCallId][operator]` for prompt/task — a commitment scheme so customers can compare N operator outputs on-chain.

With prompt/task moving to operator API, this needs a replacement:

**Option A: Signed responses.** Each operator signs their response with their Tangle key. Customer collects N signed responses and verifies off-chain. No on-chain cost.

**Option B: Optional on-chain commit.** Add a `commitResult(serviceId, jobType, resultHash)` function operators can call. Not a job — a direct contract call. Cheaper than a full job round-trip.

**Option C: Drop it.** The result hash was for auditability, but nobody queries it today. If nobody's using it, remove it.

**Recommendation: Option A.** Signed responses are cheaper, faster, and don't require on-chain transactions for every prompt. If on-chain commitment is needed later, add Option B as an enhancement.

---

## Job Registration (RegisterBlueprint.s.sol)

### New job list (7 jobs)

```solidity
Types.Job[] memory jobs = new Types.Job[](7);

jobs[0] = _makeJob("sandbox_create",     "Create a new AI sandbox",              ...);
jobs[1] = _makeJob("sandbox_delete",     "Delete an AI sandbox",                 ...);
jobs[2] = _makeJob("workflow_create",    "Create or update a workflow",           ...);
jobs[3] = _makeJob("workflow_trigger",   "Trigger a workflow execution",          ...);
jobs[4] = _makeJob("workflow_cancel",    "Cancel an active workflow",             ...);
jobs[5] = _makeJob("provision",          "Provision operator for instance mode",  ...);
jobs[6] = _makeJob("deprovision",        "Deprovision operator instance",         ...);
```

### Job ID constants (Rust)

```rust
// Shared (all modes)
pub const JOB_SANDBOX_CREATE: u8 = 0;
pub const JOB_SANDBOX_DELETE: u8 = 1;
pub const JOB_WORKFLOW_CREATE: u8 = 2;
pub const JOB_WORKFLOW_TRIGGER: u8 = 3;
pub const JOB_WORKFLOW_CANCEL: u8 = 4;
pub const JOB_PROVISION: u8 = 5;      // Instance mode only
pub const JOB_DEPROVISION: u8 = 6;    // Instance mode only
pub const JOB_WORKFLOW_TICK: u8 = 255; // Internal cron, not registered
```

Cloud binaries register routes 0-4 + 255. Instance/TEE binaries register 5-6 + applicable cloud routes.

### Backwards compatibility

This is a **breaking change** to job IDs. Since the sandbox blueprint hasn't launched to mainnet, this is fine. For any existing devnet services: redeploy.

---

## Rust Crate Changes

### sandbox-runtime (shared)

Add operator API endpoints for moved jobs:

```rust
// New routes in operator_api.rs
.route("/api/sandboxes/{id}/exec", post(sandbox_exec))
.route("/api/sandboxes/{id}/prompt", post(sandbox_prompt))
.route("/api/sandboxes/{id}/task", post(sandbox_task))
.route("/api/sandboxes/{id}/stop", post(sandbox_stop))
.route("/api/sandboxes/{id}/resume", post(sandbox_resume))
.route("/api/sandboxes/{id}/snapshot", post(sandbox_snapshot))
.route("/api/sandboxes/{id}/ssh", post(ssh_provision))
.route("/api/sandboxes/{id}/ssh", delete(ssh_revoke))

// Instance singleton variants
.route("/api/sandbox/exec", post(instance_exec))
.route("/api/sandbox/prompt", post(instance_prompt))
.route("/api/sandbox/task", post(instance_task))
.route("/api/sandbox/stop", post(instance_stop))
.route("/api/sandbox/resume", post(instance_resume))
.route("/api/sandbox/snapshot", post(instance_snapshot))
.route("/api/sandbox/ssh", post(instance_ssh_provision))
.route("/api/sandbox/ssh", delete(instance_ssh_revoke))
```

The handler logic already exists in `run_instance_exec()`, `run_instance_prompt()`, `run_instance_task()` etc. — they just need thin Axum wrappers instead of TangleLayer wrappers.

### ai-agent-sandbox-blueprint-lib (cloud)

- Remove job routes for: exec, prompt, task, stop, resume, snapshot, ssh_provision, ssh_revoke, all batch ops
- Keep routes for: sandbox_create, sandbox_delete, workflow_create, workflow_trigger, workflow_cancel, workflow_tick
- Update job ID constants

### ai-agent-instance-blueprint-lib (instance)

- Remove job routes for: exec, prompt, task, ssh_provision, ssh_revoke, snapshot
- Keep routes for: provision, deprovision
- Add auto-provision logic in binary startup (detect service, call provision_core, submit result)

### ai-agent-tee-instance-blueprint-lib (TEE instance)

- Same as instance but uses `tee_provision`/`tee_deprovision`
- Add `try_tee_backend()` (already done in sandbox-runtime for trading)

---

## Pricing Impact

Moving read-only operations off-chain means they don't have per-job pricing. Two options:

1. **Subscription covers all API usage.** Instance/TEE already use subscription pricing. This is the natural fit.
2. **Metered API billing.** Operator tracks API calls, bills via escrow keeper. More complex but more granular.

**Recommendation:** Subscription pricing. The subscription already exists for instance/TEE. For cloud, add an optional subscription tier that includes API access, or keep event-driven pricing for the remaining on-chain jobs only.

---

## Implementation Order

1. **Solidity**: Merge 3 contracts into 1 with `instanceMode`/`teeRequired` flags. Renumber jobs. Add auto-vault in `onServiceInitialized`.
2. **Rust operator API**: Add exec/prompt/task/stop/resume/snapshot/ssh endpoints to `sandbox-runtime/src/operator_api.rs`.
3. **Rust job cleanup**: Remove deprecated job routes from all 3 lib crates. Update job ID constants.
4. **Instance auto-provision**: Binary detects new service on startup, calls `provision_core`, submits result.
5. **Frontend**: Instance flow uses "Create Service" → auto-provision → configure secrets.
6. **Deploy scripts**: Set `instanceMode`/`teeRequired` flags on instance/TEE BSMs.
7. **Tests**: Solidity tests for unified contract. Rust integration tests for operator API endpoints.

---

## Files to Change

| File | Change |
|------|--------|
| `contracts/src/AgentSandboxBlueprint.sol` | Add instanceMode/teeRequired, add instance state vars, add auto-vault |
| `contracts/src/AgentInstanceBlueprint.sol` | Delete (merged into AgentSandboxBlueprint) |
| `contracts/src/AgentTeeInstanceBlueprint.sol` | Delete (merged into AgentSandboxBlueprint) |
| `contracts/script/RegisterBlueprint.s.sol` | 7 jobs instead of 17/8/8 |
| `sandbox-runtime/src/operator_api.rs` | Add exec/prompt/task/stop/resume/snapshot/ssh endpoints |
| `ai-agent-sandbox-blueprint-lib/src/lib.rs` | Update job IDs, remove deprecated routes |
| `ai-agent-sandbox-blueprint-lib/src/jobs/` | Remove exec.rs prompt/task, keep sandbox create/delete + workflow |
| `ai-agent-instance-blueprint-lib/src/lib.rs` | Update job IDs, remove deprecated routes |
| `ai-agent-instance-blueprint-lib/src/jobs/` | Remove exec/prompt/task/ssh/snapshot job handlers |
| `ai-agent-tee-instance-blueprint-lib/src/lib.rs` | Update job IDs |
| `ai-agent-tee-instance-blueprint-lib/src/jobs/` | Remove exec/prompt/task handlers |
| All 3 binary `main.rs` | Update router registrations, add auto-provision for instance |
| `scripts/deploy-local.sh` | Set instanceMode/teeRequired on appropriate BSMs |

---

## Open Questions

1. **Should stop/resume modify `sandboxActive` on-chain?** Currently they don't. If we want billing-aware stop (pause subscription), they need on-chain state. If billing is off-chain, keep them as API.

2. **Batch operations scope.** Batch create does modify state (N sandbox entries). Should it stay on-chain as a separate job, or become an API endpoint that internally submits N individual create jobs?

3. **Workflow tick.** Currently `JOB_WORKFLOW_TICK = 255` is an internal cron job. This stays as-is (not registered, internal only).
