# AI Agent Sandbox Blueprint (Cloud Mode)

Event-driven, multi-sandbox product blueprint for Tangle.

This crate owns cloud-mode job handlers and workflow orchestration. Runtime/container operations are delegated to `sandbox-runtime`.

## On-Chain Jobs (Cloud)

This crate routes only state-changing cloud jobs:

| ID | Job |
|---:|---|
| 0 | `SANDBOX_CREATE` |
| 1 | `SANDBOX_DELETE` |
| 2 | `WORKFLOW_CREATE` |
| 3 | `WORKFLOW_TRIGGER` |
| 4 | `WORKFLOW_CANCEL` |

Internal only:
- `JOB_WORKFLOW_TICK` (`255`) is a local cron job and is never registered/submitted on-chain.

Global note:
- The unified contract registers 5 total IDs (`0..4`) across all modes.

## Off-Chain Operator API

Read/operational flows are HTTP endpoints in `sandbox-runtime`:

- `/api/sandboxes/{id}/exec|prompt|task|stop|resume|snapshot|ssh|secrets|port/...`
- Auth: EIP-191 challenge-response -> PASETO session token.

## Contract Mode

Uses unified `AgentSandboxBlueprint.sol` with:
- `instanceMode=false`
- `teeRequired=false`

## Quick Checks

```bash
cargo test -p ai-agent-sandbox-blueprint-lib
REAL_SIDECAR=1 cargo test -p ai-agent-sandbox-blueprint-lib
```
