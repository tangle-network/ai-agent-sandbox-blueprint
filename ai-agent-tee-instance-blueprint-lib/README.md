# AI Agent TEE Instance Blueprint

TEE-backed variant of the [Instance Blueprint](../ai-agent-instance-blueprint-lib/README.md). Wraps the base instance blueprint with TEE enforcement — every sandbox runs inside a Trusted Execution Environment with hardware attestation.

## Overview

This blueprint reuses all handlers from the base instance blueprint (exec, prompt, task, SSH, snapshot) and enforces TEE-backed lifecycle reporting:

1. Deploy the sidecar via a `TeeBackend` (Phala, AWS Nitro, GCP, Azure, or Direct)
2. Collect hardware attestation from the TEE enclave
3. Return the attestation in the on-chain `ProvisionOutput` for client verification
4. Optionally derive a TEE-bound public key for sealed secret encryption

The on-chain contract enforces `teeRequired=true` and reverts if the operator submits an empty attestation.

## Jobs (3 on-chain)

| ID | Job | Description |
|----|-----|-------------|
| 2 | `WORKFLOW_CREATE` | Store/update workflow config |
| 3 | `WORKFLOW_TRIGGER` | Trigger workflow execution |
| 4 | `WORKFLOW_CANCEL` | Deactivate workflow |

Read-only operations (exec, prompt, task, stop, resume, snapshot, SSH) are served via the operator HTTP API, not on-chain jobs.

Canonical lifecycle sync is operator-signed direct manager reporting:
- `reportProvisioned(serviceId, sandboxId, sidecarUrl, sshPort, teeAttestationJson)`
- `reportDeprovisioned(serviceId)`

## Architecture

```
tee-instance-blueprint-lib
    │
    └── lib.rs                 ← Re-exports from instance-blueprint-lib + TEE router
            │
            └── tee_router()   → workflow jobs (2,3,4) + tick (255)
```

The `provision_core` function (from `instance-blueprint-lib`) handles the shared logic:
- Create sidecar via `create_sidecar(params, Some(backend))`
- Collect attestation from the TEE deployment
- Build `ProvisionOutput` with `tee_attestation_json` and `tee_public_key_json`

## TEE Backend Configuration

See [TEE-GUIDE.md](../TEE-GUIDE.md) for:
- Backend selection and env var reference
- Contract deployment with `teeRequired=true`
- Operator API endpoints for attestation and sealed secrets
- Client-side attestation verification
- Troubleshooting

## Testing

```bash
# Unit tests (no Docker, no TEE hardware)
cargo test -p ai-agent-tee-instance-blueprint-lib

# Integration tests (requires TEE_INTEGRATION=1)
TEE_INTEGRATION=1 cargo test -p ai-agent-tee-instance-blueprint-lib -- tee_integration
```

## Deploy

```bash
# Build
cargo build -p ai-agent-tee-instance-blueprint-bin

# Deploy contract
forge script contracts/script/DeployTeeInstance.s.sol:DeployTeeInstanceBlueprint \
  --rpc-url $RPC_URL --broadcast
```
