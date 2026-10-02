# Credentials Delivery Off-Chain — Design

Status: proposed
Author: audit follow-up to the agent-dev-container Tangle integration review
Tracking: companion issue `#credentials-off-chain`

## Problem

`sandbox_create` returns the sidecar bearer token (and the sidecar URL) inside
`SandboxCreateOutput.json`. That result is submitted on-chain via
`submitJobResult`, so **the token and URL are permanently public** in the
`JobResultSubmitted` event for anyone scanning the chain. The token is a raw
bearer credential to the sandbox's HTTP API. Network isolation of sidecars is
currently the only mitigation, and it is undocumented.

The same applies, structurally, to anything else future jobs return in results.

## Constraints

- `TangleResult<T>` uses `SolValue::abi_encode`; the operator submits whatever
  the job returns. Changing what `sandbox_create` returns changes the wire
  format consumed by the orchestrator driver, the TS SDK, and the Python SDK —
  all three pin the current encoding byte-exactly in fixture tests
  (agent-dev-container #8752), by design.
- Four implementations must move in one coordinated change; the drift guard
  makes silent partial adoption impossible, which is good.
- Existing in-flight sandboxes created under the old format must stay usable
  through a transition window.

## Non-goal

Encrypting results to the caller (ECIES to the job caller's Ethereum key via
ECDH). It works, but it changes the result format for every consumer, requires
key-management decisions (which caller key? browser wallets?), and still leaves
ciphertext on-chain for replay analysis. The operator API already has the
right primitive: **proof of being the caller**.

## Design

The operator API already authenticates "I am the on-chain creator of sandbox X"
via EIP-191 challenge sessions (`session_auth`), and `require_sandbox_owner`
binds every sandbox-scoped route to that address. Use it.

### 1. New operator endpoint (additive, no wire change)

```
GET /api/sandboxes/{sandbox_id}/credentials
Authorization: Bearer <EIP-191 session signed by the job caller>
```

Returns `{ sandbox_id, sidecar_url, token }` after
`require_sandbox_owner(sandbox_id, session.address)`.

This endpoint can ship independently and immediately: no struct change, no
migration. Clients that want off-chain credentials today can call it after
create and ignore the (still-public) on-chain token.

### 2. Opt-in on-chain redaction (wire change, coordinated)

Add a field to `SandboxCreateRequest`:

```solidity
/// 0 = legacy (token in result, default), 1 = redacted (result omits
/// sidecar_url/token; caller fetches via /credentials)
uint8 credentials_delivery;
```

- `0` (default): current behavior — full backward compatibility, existing
  clients unaffected.
- `1`: `sandbox_create` returns `json` **without** `token`/`sidecarUrl`
  (sandboxId and everything else unchanged), and the record stores the
  delivery mode. `GET /credentials` is then the only way to obtain the bearer.
- Because result arity does not change (same struct shape — the JSON string
  simply omits fields), **only the request struct arity changes**, so the
  fixture regeneration is one round across the four implementations, exactly
  the process #8752 built.

### 3. Client migration

1. Orchestrator driver: set `credentials_delivery = 1` when
   `operatorApiUrl` is configured; fetch credentials via the session it
   already needs for 2-phase secrets (#8759).
2. TS/Python SDKs: same, behind explicit opt-in
   (`credentialsDelivery: "operator-api"`).
3. After a burn-in window, flip the operator default to redacted for new
   creates and document legacy mode as deprecated.

### 4. Residual exposure and mitigations

- `env_json`/`metadata_json` remain public on-chain by design
  (non-secret data); secrets already have the off-chain 2-phase path (#8759).
- The redacted result still reveals the sandbox exists and its ID; acceptable.
- Rotate sidecar tokens on owner-request (`POST /credentials/rotate`) as a
  follow-up if token theft from the legacy window is a concern.

## Rollout checklist

- [ ] Operator: `GET /api/sandboxes/{id}/credentials` (+ tests)
- [ ] Blueprint: `credentials_delivery` field + redaction branch (+ tests)
- [ ] Fixtures regenerated from `sol!` structs; driver/TS/Python updated in one
      agent-dev-container PR (drift tests force this)
- [ ] Live end-to-end: create with delivery=1 → fetch credentials → use sandbox
- [ ] Runbook updated (docs/runbook.md) with the new flow and deprecation note
