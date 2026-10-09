# Off-chain sandbox credentials (issue #175)

Status: coordinated migration draft; **not safe to deploy until consumer gates below pass**.

## Contract

`GET /api/sandboxes/{sandbox_id}/credentials` uses the existing EIP-191/PASETO
`SessionAuth` extractor and `require_sandbox_owner` through `resolve_sandbox`.
It returns `{ "sandbox_id": "...", "sidecar_url": "...", "token": "..." }`.
Missing/invalid sessions receive 401; a different owner or ownerless record
receives 403; a missing record receives 404. The router adds `Cache-Control:
no-store` to success and failure responses. Only use a trusted operator's HTTPS
origin in production; do not derive a credential destination from untrusted
job-result JSON. Do not log response bodies, send credentials on redirects,
or persist them in public caches.

The endpoint reads the current record each time. Fetch again after resume,
secret injection/recreation, or reconnect instead of relying on an old URL/token.
An endpoint response does not itself guarantee a stopped sandbox is running.

Single-create and batch-create result JSON use a shared public-field allowlist:
`sandboxId`, `sidecarUrl`, `sshPort`, and `credentialsDelivery: "operator-api-v1"`.
Single create additionally preserves public TEE attestation/key fields. There is
no bearer field, no insecure legacy switch, and no new authentication mechanism.
URLs remain public metadata (also present in existing progress and provision
surfaces); they are not credentials.

The Solidity request shape and tuple-wrapped `(string sandboxId, string json)`
result shape are unchanged. JSON consumers still require a migration: ABI
compatibility is not behavioral compatibility. The public marker is descriptive,
not permission to trust a URL. The shared fixture is
`ai-agent-sandbox-blueprint-lib/tests/fixtures/create-result-operator-api-v1.json`.
It pins the secure JSON shape and byte-exact outer ABI for Rust, viem, and
Python eth-abi consumer tests.

## Source audit and consumer blockers

Audit baseline: blueprint `762ef4f`; ADC default branch source read 2026-10-09
(GitHub search snapshot `dc426c205ab6492da264d0d6e40edb1b80c62bd4`).

- `sandbox_create` is the registered create job. `batch_create` is currently
  unregistered but also returned a token; both now use the same projection.
- Sandbox stop/resume/delete return identifiers and booleans, no credentials.
  Operational jobs are not registered in the current router. Any future job
  registration must review arbitrary stdout/task output for secret disclosure.
- Instance/TEE instance `ProvisionOutput` and `reportProvisioned` carry public
  ID/URL/port/attestation fields, not the record token. Runtime-to-provider
  records and authenticated sidecar calls still require the private token.
- ADC `apps/orchestrator/src/driver/tangle/index.ts` create validates a token
  from the public result. `client.ts::hydrateFromChain` silently skips results
  without tokens; advancing the hydration cursor would permanently lose these
  entries. Both paths must retrieve credentials off-chain and retain pending
  sandbox/call/operator IDs on transient API failures. Do not resubmit a paid
  create when credential retrieval fails.
- ADC TS SDK `products/sandbox/sdk/src/tangle/client.ts` rejects create results
  with recognized URL but no token; its normalizer also misses `sidecarUrl`,
  allowing an unusable running entry instead. Fix URL normalization and reuse
  `getOperatorSession`/`withOperatorSession` for
  credentials before publishing a runnable entry; validate returned sandbox ID,
  URL and nonempty token. Reconnect/get must do the same.
- ADC Python SDK `products/sandbox/sdk-python/src/tangle_sandbox/tangle/client.py`
  also reads credentials from results. Its current decoder additionally assumes
  flat positional fields even though the ABI is tuple-wrapped, and its URL
  normalizer misses `sidecarUrl`. Repair those together using this fixture and
  reuse `_get_operator_session`; do not add another auth stack.
- The orchestrator has no matching session helper in its Tangle directory in
  the inspected source. Reuse the SDK's exported `OperatorSession` with the
  transaction caller's signing account, scoped to the verified responding
  operator's configured API origin. Check multi-operator placement explicitly.
- This repository's UI uses authenticated operator proxy/list APIs, does not
  extract a sidecar token from create JSON, and keeps public sidecar URLs.

## Safe rollout and rollback

1. Ship the **additive endpoint commit alone** first. This does not remediate
   public legacy tokens; do not close #175 or call this a complete security fix.
2. Migrate orchestrator create, chain hydration, TS SDK create/reconnect, and
   Python SDK with the fixture. Authenticate/preflight the configured operator
   API before a paid create. If unavailable, fail before submission. After a
   committed create, retain sandbox/call/operator IDs and retry only retrieval.
3. Verify 401 refresh-once, 403 no fallback, missing endpoint, wrong returned ID,
   multi-operator routing, token-free result, successful sidecar use, restart
   hydration, and post-resume/recreation retrieval. Missing operator API must
   yield explicit configuration refusal, never fall back to public credentials.
4. Release the redaction commit only after all supported consumers pass. Legacy
   clients must upgrade or be explicitly refused before paid creation. Binary
   release is tag/manual in `.github/workflows/release.yml`; merging these Rust
   and docs paths does not trigger the UI/image path-filtered deploy workflows.
   External operators that build from main are not covered by those workflows.
5. Verify a newly submitted public result contains no bearer on the deployed
   candidate, and the authenticated owner can use the retrieved credential.
   This requires separately authorized integration/deployment access.

Do not roll back to a credential-publishing binary. If migration fails, stop
new provisioning and repair/revert consumers while retaining the secure result
contract. Tokens already recorded on-chain cannot be erased. Existing tokens
need a separately authorized rotation/reprovisioning and network-isolation
plan; this source change neither rotates credentials nor changes live operators.

## Verification gates

- `cargo test -p sandbox-runtime credentials_require_current_owner_and_are_never_cached`
- `cargo test -p ai-agent-sandbox-blueprint-lib public_create_contract_matches_shared_fixture`
- `cargo fmt -- --check` and maintained workspace clippy/unit suites
- `SIDECAR_E2E=1 cargo test -p ai-agent-sandbox-blueprint-lib --test e2e_operator_api -- --test-threads=1`
- ADC byte-exact fixture tests in driver, TS SDK and Python SDK, plus real driver
  create/restart/reconnect/resume and negative ownership integration

A fixture decoder check alone is not a runtime security test. CI must execute
rather than skip the Docker/Anvil/sidecar tests; issue #150's TNT fixture
compatibility must be verified in the integration environment.
