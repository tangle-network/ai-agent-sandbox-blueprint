# Sandbox Blueprint Instance Architecture (Final)

Date: March 3, 2026

## Goal

Define a clean instance architecture where service initialization represents desired state, operators reconcile runtime state locally, and lifecycle state is reported on-chain directly by operators.

## Contract Surface

Single contract: `AgentSandboxBlueprint.sol` deployed in three modes.

- Cloud mode: `instanceMode=false, teeRequired=false`
- Instance mode: `instanceMode=true, teeRequired=false`
- TEE instance mode: `instanceMode=true, teeRequired=true`

On-chain jobs are state-changing only and fixed to IDs `0..4`:

- `0`: `SANDBOX_CREATE` (cloud)
- `1`: `SANDBOX_DELETE` (cloud)
- `2`: `WORKFLOW_CREATE` (cloud + instance)
- `3`: `WORKFLOW_TRIGGER` (cloud + instance)
- `4`: `WORKFLOW_CANCEL` (cloud + instance)

Instance lifecycle is not a submit-job flow.

## Instance Lifecycle Model

### 1. Service request

- User submits `requestService` with selected operators and instance config.
- Contract stores request config in `_pendingRequestConfig[requestId]`.

### 2. Service initialization

- Tangle calls `onServiceInitialized`.
- Contract stores:
  - `serviceOwner[serviceId]`
  - `serviceConfig[serviceId]`
- This marks desired state only, not runtime readiness.

### 3. Operator startup reconciliation

Each operator binary:

- loads `serviceConfig(serviceId)` + `serviceOwner(serviceId)` from chain,
- provisions local singleton sandbox if missing,
- reports lifecycle directly to manager.

### 4. Direct lifecycle reporting

Operators call manager directly:

- `reportProvisioned(serviceId, sandboxId, sidecarUrl, sshPort, teeAttestationJson)`
- `reportDeprovisioned(serviceId)`

Authorization:

- `instanceMode == true`
- `msg.sender` must be active service operator via `isServiceOperator(serviceId, msg.sender)`

TEE enforcement:

- if `teeRequired == true`, provision report must include non-empty attestation JSON.

## Why this model

- Avoids coupling lifecycle to `submitJob` permitted-caller rules.
- Uses operator-signed transactions for observed runtime facts.
- Keeps job surface minimal and stateful only.
- Works for single and multi-operator instance services.

## Runtime Responsibilities

- `sandbox-runtime`: shared Docker/TEE runtime + authenticated operator API.
- `ai-agent-instance-blueprint-lib`: instance provisioning/deprovision core, workflow handlers, startup reconciliation, direct reporting helpers.
- `ai-agent-tee-instance-blueprint-lib`: same model with TEE backend enabled.

Read/operational actions stay off-chain via operator API (`/api/sandbox/*` singleton routes).

## UI/Protocol Implications

- Instance and TEE instance blueprints expose workflow jobs only (`2,3,4`) in job UIs.
- Provision/deprovision UX is driven by service creation/termination and runtime status events, not lifecycle jobs.

## Invariants

- Repeated provision report for same `(serviceId, operator)` reverts `AlreadyProvisioned`.
- Repeated deprovision report for non-provisioned operator reverts `NotProvisioned`.
- Service membership gating is enforced on every direct lifecycle report.
