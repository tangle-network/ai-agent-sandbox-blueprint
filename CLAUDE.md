# Local Ops Memory

## Canonical Local Flow
- Run `SKIP_BUILD=1 ./scripts/deploy-local.sh` to bring up local Anvil + operator APIs and regenerate `.env.local`.
- Run `./scripts/test-e2e.sh` after deployment to validate on-chain wiring, operator APIs, auth, and lifecycle behavior.

## Integration Contract (agent-dev-container)
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
