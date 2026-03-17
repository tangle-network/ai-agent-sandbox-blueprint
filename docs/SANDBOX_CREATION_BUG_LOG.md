# Sandbox Creation Bug Log

## Stale sandbox cache after local redeployment

**Date:** 2026-03-17

**Symptoms:**
- After redeploying the local stack (`deploy-local.sh`), the UI shows sandbox entries that no longer exist on the backend.
- Interacting with these "ghost" sandboxes produces errors (404s, mismatched service IDs).
- The only workaround is manually clearing browser localStorage.

**Root cause:**
Each local deployment spins up a fresh Anvil chain with new contract addresses, service IDs, and operator state. However, the UI persists its sandbox list in localStorage under a fixed key (`sandbox_cloud_sandboxes`). When the backend is torn down and redeployed, the cached sandbox records become stale — they reference contracts and services that no longer exist — but the UI has no way to know the deployment changed and continues serving the old data.

**Impact:**
- Local development workflow friction: every redeploy requires a manual localStorage clear or the UI is broken.
- Confusing UX — the sandbox list looks populated but nothing works.

**Fix:** `cbb0e89` — fix(ui): scope sandbox localStorage cache to deployment fingerprint

## Sandbox detail page shows incorrect on-chain status for a running local sandbox

**Date:** 2026-03-17

**Symptoms:**
- A sandbox can be visibly running locally and expose a healthy sidecar URL, but the sandbox detail page still shows incorrect on-chain status.
- In the "On-Chain Status" card, `Active` may remain stuck on `Loading...`.
- In the same card, `Operator` may display `Unassigned` even though the sandbox has already been assigned on-chain.
- This creates a contradiction in the UI: the resource header and local runtime details indicate a live sandbox, while the on-chain panel suggests the sandbox is unresolved or inactive.

**Observed case:**
- Sandbox ID: `sandbox-b12e5026-80d3-4c5f-aa97-c7672cdccb78`
- Frontend showed:
  - `Active`: `Loading...`
  - `Operator`: `Unassigned`
- Chain state for the same sandbox showed:
  - `isSandboxActive(...) = true`
  - `getSandboxOperator(...) = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8`
  - `totalActiveSandboxes() = 1`

**Impact:**
- Misleading operational status in the sandbox detail view.
- Makes it harder to trust the overview page when debugging local deployments.
- Can send developers in the wrong direction by implying a provisioning or chain-state failure when the sandbox is actually live.
