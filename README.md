![Tangle Network Banner](https://raw.githubusercontent.com/tangle-network/tangle/refs/heads/main/assets/Tangle%20%20Banner.png)

# AI Agent Sandbox Blueprint

[![Discord](https://img.shields.io/badge/Discord-Join%20Chat-7289da?logo=discord&logoColor=white)](https://discord.gg/cv8EfJu3Tn)
[![Twitter](https://img.shields.io/twitter/follow/tangle_network?style=social)](https://twitter.com/tangle_network)

**AI Agent Sandbox Blueprint** is a production TEE sandbox runtime for AI agents on Tangle Network. It supports AWS Nitro, Azure, GCP, and Phala backends with sealed secrets, attestation, and managed Docker sandboxes for multi-tenant AI workloads.

## Overview

A Tangle Network blueprint that provides managed Docker sandboxes for AI agents. Operators run
sidecar containers and expose them through both on-chain jobs (lifecycle management) and an
authenticated HTTP API (read/write operations like exec, prompt, SSH, secrets).

Three deployment modes:
- **Sandbox (cloud)**: Multi-tenant fleet — callers create/delete sandboxes on-demand via on-chain jobs
- **Instance**: Single sandbox per service — auto-provisioned on operator startup
- **TEE Instance**: Same as instance but with TEE attestation and sealed secrets

## Architecture

```
Caller ─── Tangle EVM ─── BlueprintRunner ─── Job Handlers
                                 │
                          Operator API (HTTP + PASETO auth)
                                 │
                          Docker / Docktopus ─── Sidecar Containers
```

## Layered Architecture

Canonical references:
- `docs/ARCHITECTURE.md`
- `docs/CONTRACTS.md`
- `BLUEPRINT_FACTORY_STATUS.md`

Hard rules:
- On-chain jobs **must** mutate authoritative state. No read-only jobs.
- Read-only flows **must** use `eth_call` or the off-chain operator HTTP API.

Layer boundaries:
- `microvm-blueprint` = infrastructure layer
- `sandbox-runtime` = runtime contracts/adapters layer
- `ai-agent-sandbox-blueprint`, `ai-trading-blueprints`, `openclaw-hosting-blueprint` = product layer

Dependency direction:
- Allowed: Product -> `sandbox-runtime` -> `microvm-blueprint`
- Forbidden: Product -> `microvm-blueprint` (direct), product -> product

### Crate Structure

| Crate | Role |
|-------|------|
| `sandbox-runtime` | Shared library: Docker lifecycle, operator API, session auth, rate limiting, metrics, encryption, and L1 contracts (`SandboxProvider`/`RuntimeAdapter`) |
| `ai-agent-sandbox-blueprint-lib` | Cloud/sandbox mode job handlers + workflows |
| `ai-agent-instance-blueprint-lib` | Instance mode: auto-provision + billing |
| `ai-agent-tee-instance-blueprint-lib` | TEE instance: attestation + sealed secrets |
| `*-bin` | Binary entry points (one per mode) |
| `contracts/` | Solidity BSM contract (deployed 3x with different flags) |
| `ui/` | React frontend for sandbox management |

## UI Package Boundaries

The UI uses two shared packages. Keep responsibilities strict to avoid copy/paste drift:

- `@tangle-network/blueprint-ui`:
  - Blueprint and chain infrastructure (`publicClient`, chain/address helpers, ABI exports)
  - Job/provisioning/quote utilities, infra/session/tx stores
  - Reusable cross-blueprint UI primitives and layout components
  - npm: https://www.npmjs.com/package/@tangle-network/blueprint-ui
- `@tangle-network/agent-ui`:
  - Agent chat/session rendering, run/tool timeline UI, markdown/tool previews
  - Sidecar auth/session hooks and PTY terminal integration
  - Shared lightweight UI utilities in `@tangle-network/agent-ui/primitives`
  - Published from this repo via `.github/workflows/publish-agent-ui.yml`
  - npm: https://www.npmjs.com/package/@tangle-network/agent-ui
- App-local (`ui/src/**`):
  - Sandbox-specific routes, workflows, feature copy, and product behavior
  - Sandbox-only shell/layout styling concerns

Extraction rule:
- If Sandbox UI and Arena UI carry the same non-trivial implementation (roughly 20+ lines), promote it to the appropriate shared package instead of creating a third copy.
- Recommended duplication check:
  - `npx jscpd --min-lines 8 --min-tokens 80 --format ts,tsx --ignore "**/node_modules/**,**/.next/**,**/dist/**,**/build/**" /home/drew/code/blueprint-ui/src /home/drew/code/ai-agent-sandbox-blueprint/packages/agent-ui/src /home/drew/code/ai-agent-sandbox-blueprint/ui/src /home/drew/code/ai-trading-blueprints/arena/src`

## On-Chain Jobs (5 total)

| ID | Name | Mode | Description |
|----|------|------|-------------|
| 0 | `SANDBOX_CREATE` | Cloud | Create a new sandbox container |
| 1 | `SANDBOX_DELETE` | Cloud | Delete a sandbox and clean up |
| 2 | `WORKFLOW_CREATE` | Cloud + Instance | Register a workflow template |
| 3 | `WORKFLOW_TRIGGER` | Cloud + Instance | Trigger a registered workflow |
| 4 | `WORKFLOW_CANCEL` | Cloud + Instance | Cancel an active workflow |

Internal: `JOB_WORKFLOW_TICK` (255) — cron-driven workflow scheduler, never on-chain.

### Runtime Backend Selection

Sandbox creation supports backend selection via `metadata_json.runtime_backend`:

- `docker` (default)
- `firecracker` (microVM path; requires operator runtime support)
- `tee` (forces TEE provisioning path)

UI behavior:
- "Runtime Backend" selector writes to `metadata_json.runtime_backend`.
- Selecting `tee` forces `tee_required=true`.
- Selecting `firecracker` forces `tee_required=false` (current release does not support Firecracker+TEE composition).

### Instance Lifecycle Semantics

- Canonical path is operator-signed direct reporting:
  - `reportProvisioned(serviceId, sandboxId, sidecarUrl, sshPort, teeAttestationJson)`
  - `reportDeprovisioned(serviceId)`
- Authentication is `msg.sender` + Tangle membership (`isServiceOperator(serviceId, msg.sender)`).
- `onServiceInitialized` stores desired state (`owner/config`) but does not claim runtime readiness.
- Runtime startup auto-provisions locally, then reports provision directly to manager.
- State machine remains strict:
  - report provision when already provisioned => revert `AlreadyProvisioned`
  - report deprovision when not provisioned => revert `NotProvisioned`

## Operator API (HTTP)

All data endpoints require PASETO v4 session auth (EIP-191 challenge-response).

### Authentication
- `POST /api/auth/challenge` — Get a nonce to sign
- `POST /api/auth/session` — Exchange signed challenge for PASETO token
- `DELETE /api/auth/session` — Revoke current session

### Sandbox Operations (cloud mode: `/api/sandboxes/{id}/...`)
- `GET /api/sandboxes` — List caller's sandboxes
- `GET /api/sandboxes/{id}/ports` — List exposed container ports
- `POST /api/sandboxes/{id}/exec` — Execute a command
- `POST /api/sandboxes/{id}/prompt` — Run an AI prompt
- `POST /api/sandboxes/{id}/task` — Run an AI task
- `POST /api/sandboxes/{id}/stop` — Stop a sandbox
- `POST /api/sandboxes/{id}/resume` — Resume a stopped sandbox
- `POST /api/sandboxes/{id}/snapshot` — Upload a snapshot
- `POST /api/sandboxes/{id}/ssh` — Provision SSH key
- `DELETE /api/sandboxes/{id}/ssh` — Revoke SSH key
- `POST /api/sandboxes/{id}/secrets` — Inject secrets
- `DELETE /api/sandboxes/{id}/secrets` — Wipe secrets
- `ANY /api/sandboxes/{id}/port/{port}` — Proxy to container port

### Instance Operations (instance mode: `/api/sandbox/...`)
- `GET /api/sandbox/ports` — List singleton sandbox ports
- `POST /api/sandbox/exec` — Execute a command
- `POST /api/sandbox/prompt` — Run an AI prompt
- `POST /api/sandbox/task` — Run an AI task
- `POST /api/sandbox/stop` — Stop the singleton sandbox
- `POST /api/sandbox/resume` — Resume the singleton sandbox
- `POST /api/sandbox/snapshot` — Upload a snapshot
- `POST /api/sandbox/ssh` — Provision SSH key
- `DELETE /api/sandbox/ssh` — Revoke SSH key
- `ANY /api/sandbox/port/{port}` — Proxy to singleton container port

Note: `/api/sandbox/secrets` is not currently exposed; secret provisioning is currently sandbox-scoped (`/api/sandboxes/{id}/secrets`).

### Infrastructure
- `GET /health` — Runtime backend + store health check (503 when degraded)
- `GET /readyz` — Strict readiness probe (503 unless all subsystems healthy)
- `GET /metrics` — Prometheus metrics
- `GET /api/provisions` — List provision status

## Security

- **Auth**: EIP-191 challenge-response → PASETO v4.local tokens (1h TTL)
- **Encryption**: ChaCha20-Poly1305 at-rest encryption for tokens/env in stored records
- **Container hardening**: `cap_drop ALL`, `SYS_PTRACE` only, `no-new-privileges`, `readonly_rootfs`, PID limit 512, ports bound to `127.0.0.1`
- **Rate limiting**: 3-tier (auth 10/min, write 30/min, read 120/min) with XFF spoofing prevention
- **Circuit breaker**: Per-sandbox health tracking with 30s cooldown
- **Session caps**: 10K challenges, 50K sessions max with background GC
- **SSRF protection**: Snapshot destinations validated (HTTPS/S3 only, no private IPs)

## Prerequisites

- Rust 1.88+
- Docker
- Foundry (for contracts)
- Node.js 22+ / pnpm (for UI)

## Environment Variables

### Required
- `SIDECAR_IMAGE` — Docker image for sidecar containers
- `SESSION_AUTH_SECRET` — Symmetric key for PASETO tokens and at-rest encryption

### Optional
| Variable | Default | Description |
|----------|---------|-------------|
| `SIDECAR_PUBLIC_HOST` | `127.0.0.1` | Public hostname for sidecar access |
| `SIDECAR_HTTP_PORT` | `8080` | Container HTTP port |
| `SIDECAR_SSH_PORT` | `22` | Container SSH port |
| `SIDECAR_PULL_IMAGE` | `true` | Pull image on first create |
| `REQUEST_TIMEOUT_SECS` | `30` | HTTP client timeout |
| `DOCKER_OPERATION_TIMEOUT_SECS` | `60` | Docker API call timeout |
| `OPERATOR_API_PORT` | `9090` | Operator API listen port |
| `SANDBOX_DEFAULT_IDLE_TIMEOUT` | `1800` | Idle timeout (seconds) |
| `SANDBOX_DEFAULT_MAX_LIFETIME` | `86400` | Max lifetime (seconds) |
| `SANDBOX_REAPER_INTERVAL` | `30` | Reaper check interval |
| `SANDBOX_GC_INTERVAL` | `3600` | GC interval |
| `SANDBOX_RUNTIME_BACKEND` | `docker` | Default runtime backend (`docker`, `firecracker`, `tee`) |
| `FIRECRACKER_HOST_AGENT_URL` | — | Base URL for Firecracker host-agent API (required for `runtime_backend=firecracker`) |
| `FIRECRACKER_HOST_AGENT_API_KEY` | — | Optional API key header (`x-api-key`) for host-agent |
| `FIRECRACKER_HOST_AGENT_NETWORK` | `bridge` | Network value sent in host-agent create payload |
| `FIRECRACKER_HOST_AGENT_PIDS_LIMIT` | `512` | PIDs limit sent in host-agent create payload |
| `FIRECRACKER_SIDECAR_AUTH_TOKEN` | — | Optional static sidecar auth token to persist in Firecracker records |
| `WORKFLOW_CRON_SCHEDULE` | `0 * * * * *` | Cron schedule for workflow ticks |
| `CORS_ALLOWED_ORIGINS` | `localhost only` | Comma-separated CORS origins |
| `BSM_ADDRESS` | — | BSM contract address (instance mode) |
| `HTTP_RPC_ENDPOINT` / `RPC_URL` | — | Chain RPC endpoint |

## Development

```sh
# Build
cargo build --workspace

# Test
cargo test --workspace

# Format (must use nightly)
cargo +nightly fmt

# Lint
cargo clippy --workspace --tests --examples -- -D warnings

# Solidity
forge soldeer update -d && forge build && forge test -vvv

# UI
cd ui && pnpm install && pnpm test && pnpm dev

# Local dev (skip BPM bridge)
cargo run -p ai-agent-sandbox-blueprint-bin -- run --test-mode
```

## Key Concepts

- **Blueprint**: A specification for a verifiable, decentralized service on Tangle Network. Blueprints define jobs, handle results, and manage the operator lifecycle.
- **Operator**: A node runner who registers to provide services defined by a Blueprint. Operators stake assets and earn rewards for honest execution.
- **TEE (Trusted Execution Environment)**: Hardware-isolated execution environments (such as AWS Nitro Enclaves or Intel SGX) that provide confidentiality and attestation for sensitive computations.
- **Sealed Secrets**: Encrypted data that can only be decrypted inside a TEE. Secrets are sealed using ChaCha20-Poly1305 encryption and bound to a specific enclave identity.
- **Attestation**: Cryptographic proof that code is running inside a genuine TEE. Attestation reports are verified on-chain to establish trust.
- **BlueprintRunner**: The runtime that manages the lifecycle of a Blueprint operator, including job polling, execution, and result submission.

## FAQ

### What is a Tangle Blueprint?
A **Blueprint** is a specification for an Actively Validated Service (AVS) on the Tangle Network. It defines the jobs an operator can perform, how results are verified, and what on-chain contracts govern the service lifecycle.

### What TEE backends does this sandbox support?
The sandbox supports **AWS Nitro Enclaves**, **Azure Confidential Computing**, **GCP Confidential VMs**, and **Phala Network** as TEE backends. Each backend provides hardware-level isolation and remote attestation.

### What is the difference between Sandbox and Instance modes?
**Sandbox mode** runs a multi-tenant fleet of Docker containers managed by the operator, suitable for shared workloads. **Instance mode** runs a single dedicated sandbox per service, providing stronger isolation. **TEE Instance mode** adds hardware attestation and sealed secrets on top of instance mode.

### How are secrets managed in the sandbox?
Secrets are encrypted using ChaCha20-Poly1305 and stored as sealed data. Only attested TEE enclaves with the correct identity can decrypt them. The operator API provides endpoints for secret provisioning and retrieval within authenticated sessions.

### How do I deploy this Blueprint?
Install Rust 1.88+, Docker, and Foundry. Build with `cargo build`, deploy the Solidity contracts, and register as an operator using the `cargo-tangle` CLI. See the deployment section above for detailed steps.

## License

MIT OR Apache-2.0
