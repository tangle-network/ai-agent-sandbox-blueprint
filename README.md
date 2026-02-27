# AI Agent Sandbox Blueprint

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

### Crate Structure

| Crate | Role |
|-------|------|
| `sandbox-runtime` | Shared library: Docker lifecycle, operator API, session auth, rate limiting, metrics, encryption |
| `ai-agent-sandbox-blueprint-lib` | Cloud/sandbox mode job handlers + workflows |
| `ai-agent-instance-blueprint-lib` | Instance mode: auto-provision + billing |
| `ai-agent-tee-instance-blueprint-lib` | TEE instance: attestation + sealed secrets |
| `*-bin` | Binary entry points (one per mode) |
| `contracts/` | Solidity BSM contract (deployed 3x with different flags) |
| `ui/` | React frontend for sandbox management |

## On-Chain Jobs (7 total)

| ID | Name | Mode | Description |
|----|------|------|-------------|
| 0 | `SANDBOX_CREATE` | Cloud | Create a new sandbox container |
| 1 | `SANDBOX_DELETE` | Cloud | Delete a sandbox and clean up |
| 2 | `WORKFLOW_CREATE` | Cloud | Register a workflow template |
| 3 | `WORKFLOW_TRIGGER` | Cloud | Trigger a registered workflow |
| 4 | `WORKFLOW_CANCEL` | Cloud | Cancel an active workflow |
| 5 | `PROVISION` | Instance | Report auto-provision result on-chain |
| 6 | `DEPROVISION` | Instance | Report deprovision result on-chain |

Internal: `JOB_WORKFLOW_TICK` (255) — cron-driven workflow scheduler, never on-chain.

## Operator API (HTTP)

All data endpoints require PASETO v4 session auth (EIP-191 challenge-response).

### Authentication
- `POST /api/auth/challenge` — Get a nonce to sign
- `POST /api/auth/session` — Exchange signed challenge for PASETO token
- `DELETE /api/auth/session` — Revoke current session

### Sandbox Operations (cloud mode: `/api/sandboxes/{id}/...`)
- `GET /api/sandboxes` — List caller's sandboxes
- `POST /api/sandboxes/{id}/exec` — Execute a command
- `POST /api/sandboxes/{id}/prompt` — Run an AI prompt
- `POST /api/sandboxes/{id}/task` — Run an AI task
- `POST /api/sandboxes/{id}/stop` — Stop a sandbox
- `POST /api/sandboxes/{id}/resume` — Resume a stopped sandbox
- `POST /api/sandboxes/{id}/snapshot` — Upload a snapshot
- `POST /api/sandboxes/{id}/ssh/provision` — Provision SSH key
- `POST /api/sandboxes/{id}/ssh/revoke` — Revoke SSH key
- `POST /api/sandboxes/{id}/secrets` — Inject secrets
- `DELETE /api/sandboxes/{id}/secrets` — Wipe secrets

### Instance Operations (instance mode: `/api/sandbox/...`)
Same operations as above but scoped to the singleton instance sandbox.

### Infrastructure
- `GET /health` — Docker + store health check
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

## License

MIT OR Apache-2.0
