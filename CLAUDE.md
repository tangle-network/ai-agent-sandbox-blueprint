# Local Ops Memory

## Commit + PR Hygiene
- Never include `Co-Authored-By:` trailers, "Generated with" footers, or any other AI-attribution lines in commit messages or PR bodies. Apply across every repo, every branch.
- Commit messages and PR descriptions stand on their content; authorship is git's job.

## Canonical Local Flow
- Run `SKIP_BUILD=1 ./scripts/deploy-local.sh` to bring up local Anvil + operator APIs and regenerate `.env.local`.
- Run `./scripts/test-e2e.sh` after deployment to validate on-chain wiring, operator APIs, auth, and lifecycle behavior.

## Integration Contract
- `deploy-local.sh` is the source of truth for orchestrator compatibility vars in `.env.local`:
  - `TANGLE_RPC_URL`
  - `TANGLE_WS_URL`
  - `TANGLE_SERVICE_ID`
  - `TANGLE_PRIVATE_KEY`
  - `TANGLE_BLUEPRINT_CONTRACT_ADDRESS`
  - `TANGLE_E2E_IMAGE`
- Do not hand-edit these values in `.env.local`; regenerate by re-running `deploy-local.sh`.

## Reliability Do/Don't
- Do health-check both RPC and operator API before assuming local stack is usable.
- Do keep default local ports (`8645`, `9100`, `9200`) unless there is a port collision.
- Do treat the on-chain blueprint surface as 5 jobs (`0..4`) for local e2e validation.
- Do treat instance direct lifecycle reporting as canonical (`reportProvisioned` / `reportDeprovisioned`).
- Don't treat an existing `.env.local` as proof services are running.
- Don't test sandbox/instance exec via on-chain `submitJob` in local e2e; validate those via runtime/operator API integration paths.
- Don't skip `test-e2e.sh` when changing deploy scripts, service registration, or API auth flows.

## Naming Policy
- Treat the current architecture as greenfield; do not introduce new identifiers using `legacy`.
- Use canonical ingress auth env keys from `sandbox-runtime`: `SANDBOX_UI_AUTH_MODE` and `SANDBOX_UI_BEARER_TOKEN`.
- When compatibility aliases are required for external images, scope them in product crates and name them with `COMPAT` (for example `*_COMPAT_*`).

## Verified Invariants (Do Not Regress)
- Sandbox identity is immutable across secrets inject/wipe recreation. Preserve the same `sandbox_id`.
- `stop` and `resume` are idempotent API actions. "already stopped/running" must return success behavior, not 500.
- Circuit breaker is sandbox-scoped, not endpoint-scoped. After successful `resume`, clear breaker state for that sandbox.
- `runtime_backend=firecracker` is a real lifecycle path through the in-process `microvm-runtime` driver. The operator is the Firecracker host — there is no separate "host-agent" service.
- Firecracker records must persist `metadata_json.runtime_backend="firecracker"` so stop/resume/delete/reconcile route correctly.
- Live sessions are strictly owner+scope isolated:
  - sandbox scope: `sandbox:{sandbox_id}`
  - instance scope: `instance:{sandbox_id}`
- Proxied operator API payload/response contract differs from direct sidecar:
  - prompt request uses `message`
  - task request uses `prompt`
  - task response uses `result`
- In proxied mode, if `session_id` is missing, create live chat session first, then invoke prompt/task.

## Verified Flow Notes (E2E Expectations)
- After secrets inject/wipe, sidecar URL may change. Always re-read URL from operator API before readiness checks.
- Stderr markers may appear in `stderr` or `stdout` depending on sidecar behavior; tests should accept either when validating command output markers.
- Snapshot destination policy currently rejects `http://` and accepts `https://` / `s3://`; e2e should validate this policy, not old behavior.
- Agent endpoints may return `502` (backend unavailable) followed by `503` (breaker cooldown). This is acceptable in optional-agent local e2e.
- Firecracker startup reads its config from `MICROVM_FIRECRACKER_*` env vars (bin / kernel / rootfs / socket dir / state dir / vcpu / mem). Misconfiguration surfaces as `Unavailable` (binary or images missing) on first create, not on operator boot.
- Firecracker create/resume currently fail with `SandboxError::Unsupported` because the `microvm-runtime 0.1.0-alpha.1` primitive does not yet expose a host-reachable sidecar endpoint, per-VM env injection, or port forwarding. These land in `microvm-runtime 0.2.0`. Stop / delete / status / reaper reconcile **are** wired end-to-end.
- Never reintroduce host-agent HTTP plumbing for firecracker. The operator process is the host.

## Regression Gate (Run Before Merge)
- `cargo test -p sandbox-runtime`
- `cargo clippy -p sandbox-runtime --all-targets --all-features -- -D warnings`
- `pnpm --dir ui test`
- `pnpm --dir ui typecheck`
- `REAL_SIDECAR=1 cargo test -p ai-agent-sandbox-blueprint-lib --test real_sidecar -- --test-threads=1`
- `REAL_SIDECAR=1 cargo test -p ai-agent-instance-blueprint-lib --test real_sidecar -- --test-threads=1`
- `SIDECAR_E2E=1 cargo test -p ai-agent-sandbox-blueprint-lib --test e2e_operator_api -- --test-threads=1`
