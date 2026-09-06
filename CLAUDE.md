# Agent Sandbox Blueprint

For architecture and contracts, read [ARCHITECTURE.md](docs/ARCHITECTURE.md) and [CONTRACTS.md](docs/CONTRACTS.md).
For operations, read [runbook.md](docs/runbook.md).

## Local integration

[deploy-local.sh](scripts/deploy-local.sh) owns local chain deployment, operator APIs, and generated `.env.local` integration values.
Run it to regenerate those values; do not hand-edit them or treat an existing file as proof of healthy services.
Skip its build only when the existing binaries match the source under test.
Check both RPC and operator health, then run [test-e2e.sh](scripts/test-e2e.sh) for deployment, service registration, or API authentication changes.
Use configured ports unless they conflict.

Keep chain jobs distinct from direct operator/runtime execution paths.
Read the routers and contract definitions for the current job surface.
Instance lifecycle reporting uses `reportProvisioned` and `reportDeprovisioned`.
Use canonical ingress authentication keys from `sandbox-runtime`; scope necessary image compatibility aliases to their product crates.

## Runtime boundaries

- Preserve `sandbox_id` across secrets injection or wipe and subsequent recreation.
  Re-read the sidecar URL after recreation before checking readiness.
- Keep stop and resume idempotent and circuit-breaker state scoped to the sandbox.
  A successful resume clears that sandbox's breaker.
- Isolate live sessions by authenticated owner and sandbox/instance scope.
- Preserve the distinct proxied prompt and task contracts.
  When a proxied request lacks a session, create its live session before invoking the operation.
- Firecracker runs through the in-process `microvm-runtime` driver; the operator is its host.
  Persist the selected backend so stop, resume, delete, and reconciliation reach the correct driver.
  Check current driver and image requirements for guest metadata, authentication, network attachments, and cleanup.
- Preserve snapshot destination validation and distinguish backend unavailability from breaker cooldown in tests.

## Verification and structure

Use [.github/workflows/ci.yml](.github/workflows/ci.yml) for the maintained runtime, UI, and real-sidecar checks.
Run the relevant real-sidecar suites for lifecycle or API changes and disclose unavailable prerequisites.
Check output and effects, not only process exit codes.

[check-file-sizes.sh](scripts/check-file-sizes.sh) enforces source limits and the existing size baseline.
Split by responsibility when a change would grow an oversized module.
Update the baseline only after a real reduction; do not raise limits to hide growth.
