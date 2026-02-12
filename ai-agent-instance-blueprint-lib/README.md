# AI Agent Instance Blueprint

Subscription-based, replicated sandbox blueprint for the Tangle Network. Each service instance is a single AI agent sandbox, independently provisioned by every operator in the service. Customers choose how many operators (1 for simple use, N for redundancy or TEE cross-verification).

## Overview

This blueprint follows a **subscription-based, multi-operator model**:

- **One sandbox per operator**: Each operator provisions and manages exactly one sandbox for the service instance. No fleet management — just a single persistent agent environment.
- **Implicit addressing**: Job requests don't include `sidecar_url` or `sidecar_token`. The operator automatically resolves its own singleton sandbox.
- **Multi-operator replication**: Customers choose N operators. Each independently provisions an identical sandbox. For prompt/task jobs, ALL operators respond — the contract stores per-operator result hashes for comparison/aggregation.
- **N sidecar URLs**: After provisioning, customers call `getOperatorEndpoints(serviceId)` on-chain to get all N operator addresses and their sidecar URLs. Each sidecar URL can be used for direct streaming output.
- **TEE attestation**: Operators can provide TEE attestation during provisioning. The contract stores attestation hashes per-operator for customer verification.

## Jobs (8 total)

| ID | Job | Description |
|----|-----|-------------|
| 0 | `PROVISION` | Create the singleton sandbox for this operator |
| 1 | `EXEC` | Execute a shell command |
| 2 | `PROMPT` | Single-turn LLM agent interaction |
| 3 | `TASK` | Multi-turn LLM agent session |
| 4 | `SSH_PROVISION` | Add SSH key |
| 5 | `SSH_REVOKE` | Remove SSH key |
| 6 | `SNAPSHOT` | Snapshot sandbox to destination |
| 7 | `DEPROVISION` | Destroy the sandbox |

## Multi-Operator Architecture

```
Customer creates service with N operators:

  Operator A ──→ provisions Sandbox A ──→ sidecar_url_a
  Operator B ──→ provisions Sandbox B ──→ sidecar_url_b
  Operator C ──→ provisions Sandbox C ──→ sidecar_url_c

Customer calls getOperatorEndpoints(serviceId):
  → [addr_a, addr_b, addr_c], [url_a, url_b, url_c]

For EXEC/SSH/SNAPSHOT (any single operator suffices):
  → Tangle routes to one operator, returns result

For PROMPT/TASK (all operators must respond):
  → Tangle collects N results
  → Contract stores keccak256(output) per operator
  → Customer calls getJobResultHashes(serviceId, jobCallId)
    → compares hashes to verify consistency across operators
  → Customer can stream from each sidecar URL independently
```

### Why multi-operator?

1. **Redundancy**: If one operator goes down, others keep serving.
2. **TEE cross-verification**: Run the same prompt on N TEE-enabled operators, compare attestations and results to verify confidential execution.
3. **Consensus**: For high-stakes AI tasks, compare N independent results. If they agree, confidence is high.
4. **Streaming**: Customer gets N sidecar URLs and can stream output from each operator in real-time.

### Reading results

The contract provides two key view functions:

```solidity
// Get all operator addresses + sidecar URLs
function getOperatorEndpoints(uint64 serviceId)
    returns (address[] operators, string[] sidecarUrls);

// Get per-operator result hashes for a prompt/task job call
function getJobResultHashes(uint64 serviceId, uint64 jobCallId)
    returns (address[] operators, bytes32[] resultHashes);
```

Customers can also listen for `OperatorResultSubmitted` events:

```solidity
event OperatorResultSubmitted(
    uint64 indexed serviceId,
    uint64 indexed jobCallId,
    address indexed operator,
    uint8 job,
    bytes32 resultHash
);
```

## Pricing (6 tiers)

| Tier | Multiplier | Jobs |
|------|-----------|------|
| 1x | Trivial | EXEC, SSH_REVOKE, DEPROVISION |
| 2x | Light state | SSH_PROVISION |
| 5x | I/O-heavy | SNAPSHOT |
| 20x | Single LLM call | PROMPT |
| 50x | Container lifecycle | PROVISION |
| 250x | Multi-turn agent | TASK |

## When to use this blueprint

Choose the **Instance Blueprint** when you need:

- A dedicated, always-on AI agent per customer
- Multi-operator redundancy or TEE verification
- Simplified addressing (no URL/token management)
- Subscription-based billing model
- Result comparison across independent operators
- Streaming from N operator endpoints

## Comparison with Sandbox Blueprint

| Feature | Instance Blueprint | Sandbox Blueprint |
|---------|-------------------|------------------|
| Model | Subscription-based, 1:1 | Event-driven, multi-tenant |
| Sandboxes per operator | One (singleton) | Many (fleet) |
| Addressing | Implicit (operator auto-resolves) | Explicit `sidecar_url` + `sidecar_token` |
| Multi-operator | All N operators run identical copies, respond independently | Each sandbox assigned to 1 operator via capacity-weighted selection |
| Operator selection | Customer chooses N operators at service creation | Capacity-weighted assignment at sandbox creation |
| Result aggregation | Prompt/task: all N operators respond, contract stores per-operator result hashes | One operator responds per sandbox |
| Batch/Workflow | No | Yes |
| On-chain state | Operator endpoints, result hashes, TEE attestations | Sandbox registry, capacity, workflows |
| Best for | Dedicated agents, TEE verification, consensus | Platforms, dev tools, CI/CD |

## Smart Contract

`AgentInstanceBlueprint.sol` — tracks per-operator provisioning status, sidecar URLs, TEE attestation hashes, and per-operator result hashes for prompt/task aggregation.

Key on-chain state:
- `serviceOperators[serviceId]` — enumerable list of provisioned operators
- `operatorSidecarUrl[serviceId][operator]` — each operator's sidecar URL
- `operatorAttestationHash[serviceId][operator]` — TEE attestation hash
- `jobResultHash[serviceId][jobCallId][operator]` — per-operator result hash

## Testing

```bash
# Unit + wiremock integration tests
cargo test -p ai-agent-instance-blueprint-lib

# All tests including ABI roundtrip, helper functions, and instance state
cargo test -p ai-agent-instance-blueprint-lib -- --nocapture
```

## Deploy

```bash
# Deploy the Blueprint Service Manager contract
forge script contracts/script/DeployInstance.s.sol:DeployInstanceBlueprint \
  --rpc-url $RPC_URL --broadcast

# Configure per-job pricing (after blueprint registration on Tangle)
BASE_RATE=1000000000000000 \
BLUEPRINT_ID=<id> \
TANGLE_ADDRESS=<proxy> \
BSM_ADDRESS=<bsm> \
forge script contracts/script/ConfigureInstanceJobRates.s.sol:ConfigureInstanceJobRates \
  --rpc-url $RPC_URL --broadcast
```
