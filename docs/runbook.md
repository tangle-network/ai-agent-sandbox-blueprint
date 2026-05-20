# Operator Runbook

This runbook covers operating the AI agent sandbox blueprints in production:
deployment, configuration, key handling, and incident response. For
architecture, see `ARCHITECTURE.md`. For contract-level details, see
`CONTRACTS.md`.

The blueprint family ships three operator variants:

- **sandbox** (`ai-agent-sandbox-blueprint-bin`) — cloud-mode operator that
  hosts ephemeral sidecar containers via Docker.
- **instance** (`ai-agent-instance-blueprint-bin`) — single-tenant operator
  that runs one sandbox per service. Reports lifecycle directly via
  `reportProvisioned` / `reportDeprovisioned`.
- **TEE instance** (`ai-agent-tee-instance-blueprint-bin`) — confidential
  variant that brings up sandboxes inside a TEE (Phala dstack, AWS Nitro,
  GCP Confidential Space, Azure SKR, or operator-managed direct hardware).

---

## 1. Required environment per variant

### Common to all variants

| Var | Purpose | Required |
|---|---|---|
| `KEYSTORE_URI` | Path or URI of the operator keystore (e.g. `file:///var/lib/tangle/keystore`) | yes |
| `HTTP_RPC_ENDPOINT` (alias `RPC_URL`) | Tangle EVM RPC endpoint | yes |
| `TANGLE_WS_URL` | Tangle WS endpoint for event subscription | yes |
| `BLUEPRINT_STATE_DIR` | Directory for persistent operator state (sandbox records, chat sessions) | yes |
| `SESSION_AUTH_SECRET` | 32+ byte secret used to derive PASETO + secrets-at-rest encryption keys. Sessions and stored secrets do **not** survive restart without it. | **production: yes** |
| `SANDBOX_UI_AUTH_MODE`, `SANDBOX_UI_BEARER_TOKEN` | UI ingress auth (canonical names — do not rename) | yes |

### Sandbox-mode only

| Var | Purpose |
|---|---|
| `OPERATOR_API_PORT` | Port the operator API binds to (default `9100`) |
| `PUBLIC_HOST` | Externally-reachable hostname (operators behind NAT/VPN should set explicitly; auto-detect via Tailscale IPv4 with `AUTO_DETECT_PUBLIC_HOST=1`) |
| `SIDECAR_IMAGE` | Container image used for sandboxes (default `blueprint-sidecar:all-harness` for dev) |

### Firecracker (microVM) backend

Set when running sandboxes on a Firecracker host instead of Docker:

| Var | Purpose | Required |
|---|---|---|
| `FIRECRACKER_HOST_AGENT_URL` (alias `HOST_AGENT_URL`) | Host-agent endpoint (e.g. `http://10.0.0.5:8080`) | yes |
| `FIRECRACKER_HOST_AGENT_API_KEY` (alias `HOST_AGENT_API_KEY`) | Bearer for host-agent | recommended |
| `FIRECRACKER_HOST_AGENT_NETWORK` | Bridge network name | optional |
| `FIRECRACKER_HOST_AGENT_PIDS_LIMIT` | PID cap per microVM (default 512) | optional |
| `FIRECRACKER_SIDECAR_AUTH_DISABLED` + `FIRECRACKER_SIDECAR_AUTH_TOKEN` | **Mutually exclusive — exactly one must be set.** Set `_DISABLED=true` for trusted single-tenant host or `_TOKEN=<bytes>` for shared-host auth. | yes (one of) |

### TEE instance only

| Var | Purpose | Required |
|---|---|---|
| `TEE_BACKEND` | One of `phala`, `nitro`, `aws`, `gcp`, `azure`, `direct` | yes |
| `PHALA_API_KEY` | Phala dstack API key | for `phala` |
| `PHALA_API_ENDPOINT` | Phala API endpoint override | optional |
| `AWS_REGION`, `AWS_NITRO_*` | AWS Nitro config | for `nitro`/`aws` |
| `GCP_PROJECT_ID`, `GCP_ZONE`, etc. | GCP Confidential Space config | for `gcp` |
| `AZURE_SUBSCRIPTION_ID`, etc. | Azure SKR config | for `azure` |
| `TEE_DIRECT_TYPE` | `tdx` / `sev` / `nitro` | for `direct` |
| `TEE_ATTESTATION_NONCE` | 32–64 byte hex deploy-time attestation nonce | optional |

### QoS / heartbeat (optional)

| Var | Purpose |
|---|---|
| `QOS_DRY_RUN` | `true` to skip heartbeat reports (default `true` until ops sign-off) |
| `QOS_METRICS_INTERVAL_SECS` | Default 60 |
| `STATUS_REGISTRY_ADDRESS` | On-chain status registry — required for live heartbeats |
| `BLUEPRINT_ID`, `SERVICE_ID` | Required for heartbeats to identify the operator |

### Observability

| Var | Purpose |
|---|---|
| `LOKI_PUSH_URL` | Optional Loki ingest endpoint |
| `RUST_LOG` | `tracing` filter (default `info`) |

---

## 2. Local deployment (Anvil + Tangle local-testnet)

`scripts/deploy-local.sh` is the **source of truth** for local orchestrator
compatibility. Do not hand-edit `.env.local`; regenerate by re-running
the script.

```bash
# Bring up local Anvil + register blueprints + start operator APIs
SKIP_BUILD=1 ./scripts/deploy-local.sh

# Validate end-to-end wiring (auth, lifecycle, on-chain reporting)
./scripts/test-e2e.sh
```

**Default ports** (kept distinct from sibling repos):

- Anvil: `8645`
- Sandbox operator API: `9100`
- Instance operator API: `9200`
- TEE instance operator API: `9300` (when `ENABLE_TEE_OPERATOR=1`)

A passing `test-e2e.sh` is the regression gate before merging changes to
deploy scripts, service registration, or API auth.

---

## 3. Production deployment

### 3.1 Deploy the contracts

The blueprint contract surface is `AgentSandboxBlueprint.sol` (cloud-mode
selector + lifecycle hooks) plus `OperatorSelection.sol` (operator
selection helper).

```bash
export RPC_URL=<chain rpc>
export PRIVATE_KEY=<deployer key>
export ETHERSCAN_KEY=<chain explorer api key>

forge script contracts/script/Deploy.s.sol \
  --rpc-url $RPC_URL --broadcast --slow \
  --verify --etherscan-api-key $ETHERSCAN_KEY
```

For instance / TEE-instance operator variants, also run
`DeployInstance.s.sol` / `DeployTeeInstance.s.sol`.

### 3.2 Register the blueprint with Tangle

```bash
forge script contracts/script/RegisterBlueprint.s.sol \
  --rpc-url $RPC_URL --broadcast --slow
```

Capture the resulting `blueprint_id`. The operator binary needs it via
`BLUEPRINT_ID` for heartbeats and on-chain identity.

### 3.3 Configure pricing

```bash
forge script contracts/script/ConfigureJobRates.s.sol \
  --rpc-url $RPC_URL --broadcast
```

### 3.4 Provision the operator

1. Generate / import an operator keystore (`cargo tangle key import`).
2. Set the env vars from §1 above. **`SESSION_AUTH_SECRET` is required in
   production** — without it, sessions and at-rest-encrypted secrets are
   re-keyed on every restart.
3. Start the operator binary as a systemd unit (or equivalent supervised
   process). The operator must restart cleanly on crash; sessions and
   sealed secrets persist across restarts when `SESSION_AUTH_SECRET` is
   stable.
4. Confirm health:

   ```bash
   curl -fsS http://127.0.0.1:$OPERATOR_API_PORT/api/auth/challenge
   # expect 200 with a JSON nonce
   ```

5. Watch for heartbeat success in operator logs (or set `QOS_DRY_RUN=false`
   once heartbeats are wired and `STATUS_REGISTRY_ADDRESS` is set).

---

## 4. Adding a new chain

For each new EVM chain we want to support:

1. **Pick a chain ID.** Confirm it's not already in the deployments
   manifest.

2. **Set deploy env:**

   ```bash
   export CHAIN_ID=<id>
   export RPC_URL=<chain rpc>
   export PRIVATE_KEY=<deployer key>
   export ETHERSCAN_KEY=<chain explorer api key>
   ```

3. **Deploy contracts:**

   ```bash
   forge script contracts/script/Deploy.s.sol \
     --rpc-url $RPC_URL --broadcast --slow \
     --verify --etherscan-api-key $ETHERSCAN_KEY
   ```

4. **Register blueprint:**

   ```bash
   forge script contracts/script/RegisterBlueprint.s.sol \
     --rpc-url $RPC_URL --broadcast --slow
   ```

5. **Capture deployment artifacts.** Record the deployed contract
   addresses + `blueprint_id` in the chain's secrets manager template
   (operator env vars). The operator binary needs:

   - `TANGLE_BLUEPRINT_CONTRACT_ADDRESS`
   - `BLUEPRINT_ID`
   - `STATUS_REGISTRY_ADDRESS` (if heartbeats are enabled)

6. **Wire UI chain registry.** Update `ui/src/lib/chains/` with the new
   chain entry so the operator UI can target it.

7. **Provision an operator** following §3.4 above against the new chain.

8. **Smoke test** an end-to-end sandbox lifecycle (create → prompt →
   delete) before opening to users.

---

## 5. Operator key rotation

Operator keys are managed by `cargo-tangle` against the `KEYSTORE_URI`.
Rotation procedure:

1. **Stop the operator** (graceful shutdown — running sandboxes will be
   reconciled on restart).
2. **Generate the new key** (`cargo tangle key generate`) and import into
   the keystore directory.
3. **Update the on-chain operator record** to point at the new public
   key via the appropriate Tangle precompile call.
4. **Restart the operator.** Watch for `signed challenge accepted` in
   logs and confirm a heartbeat round-trips.
5. **Revoke the old key** in the keystore (delete the entry once the new
   key is confirmed working).

`SESSION_AUTH_SECRET` is **independent** of the operator signing key — it
controls PASETO session token encryption and at-rest secrets. Rotating
the operator key does **not** require rotating `SESSION_AUTH_SECRET`. If
`SESSION_AUTH_SECRET` itself is compromised, all active sessions and
stored secret material must be invalidated:

1. Stop the operator.
2. Set the new `SESSION_AUTH_SECRET`.
3. Wipe `BLUEPRINT_STATE_DIR/sandboxes` to invalidate sealed `user_env_json`
   entries (they were sealed under the old key).
4. Restart. Users must re-authenticate and re-inject their secrets.

---

## 6. Common failure modes

### Operator API returns 503 with `circuit_breaker`

The sandbox-scoped circuit breaker has tripped after repeated upstream
failures. Resolution:

```bash
curl -X POST http://127.0.0.1:$OPERATOR_API_PORT/api/sandboxes/$ID/resume \
  -H "Authorization: Bearer $TOKEN"
```

The breaker clears on successful resume.

### Firecracker create fails with `validation` error

Either `FIRECRACKER_HOST_AGENT_URL` is unset, or the sidecar auth mode is
ambiguous. Confirm exactly one of `_DISABLED=true` or `_TOKEN=<bytes>` is
set (see §1).

### TEE attestation rejected

The `TEE_ATTESTATION_NONCE` must be a 32–64 byte hex string. Phala dstack
also requires the `PHALA_API_KEY` to have provisioning rights for the
target TEE region — check the Phala dashboard.

### Heartbeats not reaching the registry

`STATUS_REGISTRY_ADDRESS` must be set. `QOS_DRY_RUN` must be `false`.
`BLUEPRINT_ID` and `SERVICE_ID` must match the on-chain registration. Set
`RUST_LOG=blueprint_qos=debug` for verbose heartbeat tracing.

### `SESSION_AUTH_SECRET is not set` warning at boot

Set the env var and restart. **Do not** silence this warning in production
— without a stable secret, every restart invalidates all sessions and
makes stored sandbox secrets unreadable.

---

## 7. Reference: where things live

- **Contracts**: `contracts/src/{AgentSandboxBlueprint,OperatorSelection}.sol`
- **Deploy scripts**: `contracts/script/{Deploy,DeployInstance,DeployTeeInstance,RegisterBlueprint,ConfigureJobRates}.s.sol`
- **Operator binaries**: `ai-agent-{sandbox-blueprint,instance-blueprint,tee-instance-blueprint}-bin/`
- **Shared runtime**: `sandbox-runtime/`
- **Local deploy / e2e scripts**: `scripts/{deploy-local.sh,test-e2e.sh,fetch-localtestnet-fixtures.sh}`
- **Architecture overview**: `docs/ARCHITECTURE.md`
- **Contract surface**: `docs/CONTRACTS.md`
- **Benchmarks**: `docs/BENCHMARKS.md`
- **Local ops memory**: `CLAUDE.md` (regression gate + invariants)
