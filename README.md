# AI Agent Sandbox Blueprint

## Overview

This blueprint exposes the Agent Dev Container sandbox API surface as Tangle EVM jobs. Operators
run the agent-dev-container stack, and on-chain callers trigger sandbox lifecycle and sidecar
execution through this blueprint.

The template was created with `cargo tangle blueprint create --tangle`, then extended to map the
sandbox SDK endpoints into on-chain callable jobs.

## Features

- Sandbox lifecycle: create, stop, resume, delete
- Sidecar execution: `/exec` and `/agents/run` with auth passthrough

## Prerequisites

- Rust 1.88+ (see `rust-toolchain.toml`)
- Foundry (for contracts)
- `cargo-tangle` from the `v2` branch
- Access to `agent-dev-container` (branch `feat/billing-gateway`)

## Environment

Set these to point at your running agent-dev-container services:

- `SANDBOX_API_BASE_URL` (default: `https://agents.tangle.network`)
- `SANDBOX_API_KEY` (required for sandbox jobs unless callers pass `auth_token`)
- `SIDECAR_TOKEN` (optional default for sidecar jobs)
- `REQUEST_TIMEOUT_SECS` (default: `30`)

## Job Map

Sandbox jobs (write-only):
- `JOB_SANDBOX_CREATE` (0)
- `JOB_SANDBOX_DELETE` (3)
- `JOB_SANDBOX_STOP` (4)
- `JOB_SANDBOX_RESUME` (5)
- `JOB_SANDBOX_EXEC` (6)
- `JOB_SANDBOX_PROMPT` (7)

## Development

Build the project:

```sh
cargo build --workspace --all-features
```

Run tests:

```sh
cargo test --workspace --all-features
```

Deploy the blueprint to a devnet:

```sh
cargo tangle blueprint deploy tangle --network devnet
```

## License

Licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
