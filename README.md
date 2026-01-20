# AI Agent Sandbox Blueprint

## Overview

This blueprint exposes the sidecar container API surface as Tangle EVM jobs. Operators provide
compute by running sidecar containers locally (via Docktopus/Docker). Callers trigger write-only
jobs on-chain and receive results off-chain through the blueprint runtime.

## Features

- Sandbox lifecycle: create, stop, resume, delete, snapshot
- Sidecar execution: `/exec` and `/agents/run`
- Batch execution: create, exec, task, collect
- Workflows: create, trigger, cancel (on-chain registry + cron tick)
- SSH access: provision/revoke via sidecar exec

## Prerequisites

- Rust 1.88+ (see `rust-toolchain.toml`)
- Docker (operator runtime)
- Foundry (for contracts)
- `cargo-tangle` from the `v2` branch
- Access to the sidecar image (`SIDECAR_IMAGE`)

## Environment

- `SIDECAR_IMAGE` (default: `ghcr.io/tangle-network/sidecar:latest`)
- `SIDECAR_PUBLIC_HOST` (default: `127.0.0.1`)
- `SIDECAR_HTTP_PORT` (default: `8080`)
- `SIDECAR_SSH_PORT` (default: `22`)
- `SIDECAR_PULL_IMAGE` (default: `true`)
- `DOCKER_HOST` (optional docker socket override)
- `REQUEST_TIMEOUT_SECS` (default: `30`)
- `WORKFLOW_CRON_SCHEDULE` (default: `0 * * * * *`)

## Job Map

Sandbox jobs (write-only):
- `JOB_SANDBOX_CREATE` (0)
- `JOB_SANDBOX_STOP` (1)
- `JOB_SANDBOX_RESUME` (2)
- `JOB_SANDBOX_DELETE` (3)
- `JOB_SANDBOX_SNAPSHOT` (4)

Execution jobs:
- `JOB_EXEC` (10)
- `JOB_PROMPT` (11)
- `JOB_TASK` (12)

Batch jobs:
- `JOB_BATCH_CREATE` (20)
- `JOB_BATCH_TASK` (21)
- `JOB_BATCH_EXEC` (22)
- `JOB_BATCH_COLLECT` (23)

Workflow jobs:
- `JOB_WORKFLOW_CREATE` (30)
- `JOB_WORKFLOW_TRIGGER` (31)
- `JOB_WORKFLOW_CANCEL` (32)
Internal workflow scheduler:
- `JOB_WORKFLOW_TICK` (33)

SSH jobs:
- `JOB_SSH_PROVISION` (40)
- `JOB_SSH_REVOKE` (41)

## Operator Selection

Use `previewOperatorSelection(count, seed)` on the blueprint contract to select eligible operators
and pass the operator list plus encoded `SelectionRequest` in `requestInputs` when calling
`requestService`.

## Development

Build the project:

```sh
cargo build --workspace --all-features
```

Run tests:

```sh
cargo test --workspace --all-features
```
