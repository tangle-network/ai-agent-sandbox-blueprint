# AI Agent Instance Blueprint

Subscription-style singleton sandbox blueprint (one sandbox per operator per service).

This crate owns instance-mode provisioning/deprovisioning logic and auto-provision startup behavior. Read/operational actions are served by `sandbox-runtime` operator API.

## On-Chain Jobs (Instance)

This crate routes state-changing instance jobs:

| ID | Job |
|---:|---|
| 2 | `WORKFLOW_CREATE` |
| 3 | `WORKFLOW_TRIGGER` |
| 4 | `WORKFLOW_CANCEL` |

Global note:
- The unified contract registers `0..4` for all modes.
- Fleet-only sandbox lifecycle jobs (`0..1`) are not routed by this crate.
- Internal cron tick is `JOB_WORKFLOW_TICK` (`255`) and is not on-chain.

## Provisioning Model

- Auto-provision reads `serviceConfig(serviceId)` and `serviceOwner(serviceId)` from the BSM and provisions locally.
- Canonical sync path is direct manager reporting by the operator signer:
  - `reportProvisioned(serviceId, sandboxId, sidecarUrl, sshPort, teeAttestationJson)`
  - `reportDeprovisioned(serviceId)`

Lifecycle semantics:
- Contract state is strict:
  - Repeated provision reports revert with `AlreadyProvisioned`.
  - Repeated deprovision reports revert with `NotProvisioned`.
- Direct report auth uses on-chain membership (`isServiceOperator`) + `msg.sender`.

## Off-Chain Operator API

Instance endpoints (singleton):

- `/api/sandbox/exec|prompt|task|stop|resume|snapshot|ssh|port/...`
- `/api/sandbox/secrets` is not currently exposed.

## Contract Mode

Uses unified `AgentSandboxBlueprint.sol` with:
- `instanceMode=true`
- `teeRequired=false`

## Quick Checks

```bash
cargo test -p ai-agent-instance-blueprint-lib
```
