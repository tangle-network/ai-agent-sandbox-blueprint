# AI Agent Sandbox Blueprint - Design

## Summary

This blueprint is a sidecar-only model. Operators provide compute by running sidecar containers
locally via Docker or inside Trusted Execution Environments (TEE). The blueprint runtime provisions
containers, returns a per-sandbox bearer token, and proxies write-only job calls to the sidecar API.
No centralized orchestrator is required or used.

When `TEE_BACKEND` is configured, sandboxes requesting `tee_required: true` are deployed inside
hardware-backed enclaves (Intel TDX, AWS Nitro, AMD SEV-SNP) with attestation and sealed secret
support. Non-TEE requests continue through the standard Docker path.

The sidecar container runs as a non-root user with `/home/agent` as the primary workspace directory.

## Architecture

```
┌─────────────┐     JobSubmitted      ┌───────────────────────┐
│   Tangle    │ ───────────────────── │  Blueprint Runner      │
│  (on-chain) │ ◄─────────────────── │  (Rust binary)         │
└─────────────┘     JobResult         │                        │
                                      │  ┌──────────────────┐  │
                                      │  │     Router       │  │
                                      │  │    (18 jobs)     │  │
                                      │  └────────┬─────────┘  │
                                      │           │            │
                                      │  ┌────────┴─────────┐  │
                                      │  │     Runtime      │  │
                                      │  │  (Docker / TEE)  │  │
                                      │  └──┬───────────┬───┘  │
                                      │     │           │      │
                                      │  ┌──┴──┐   ┌────┴───┐  │
                                      │  │Reap │   │Operator│  │
                                      │  │GC   │   │  API   │  │
                                      │  │Work │   │(secrets│  │
                                      │  │Metr │   │ + TEE) │  │
                                      │  └─────┘   └────────┘  │
                                      └─────┬───────────┬──────┘
                                            │           │
                                     Docker API    Cloud API
                                            │      (optional)
                                      ┌─────┴─────┐  ┌─────┴──────┐
                                      │  Docker    │  │ TEE Enclave│
                                      │ Containers │  │  (Phala,   │
                                      │(per-sandbox│  │ AWS, GCP,  │
                                      │            │  │ Azure)     │
                                      └────────────┘  └────────────┘
```

## Module Structure

| Module | Purpose |
|--------|---------|
| `lib.rs` | Public API surface, job routing, ABI type definitions |
| `runtime.rs` | Docker/TEE container lifecycle, sandbox state machine, config |
| `reaper.rs` | Idle/lifetime enforcement, tiered garbage collection |
| `workflows.rs` | Cron-scheduled workflow execution engine |
| `metrics.rs` | Atomic counters for on-chain QoS reporting |
| `http.rs` | Sidecar HTTP client helpers (auth, JSON posting) |
| `auth.rs` | Token generation and validation |
| `session_auth.rs` | EIP-191 challenge/response + PASETO session tokens |
| `rate_limit.rs` | Per-IP sliding-window rate limiting for operator API |
| `error.rs` | `SandboxError` enum (Auth, Docker, Http, Validation, NotFound, Storage, CloudProvider) |
| `store.rs` | Persistent storage bridge (LocalDatabase) |
| `util.rs` | JSON parsing, shell escaping, snapshot command builder |
| `operator_api.rs` | Axum REST API for sandbox listing, provision progress, secrets, sealed secrets |
| `secret_provisioning.rs` | 2-phase plaintext secret injection (recreate container with merged env) |
| `provision_progress.rs` | Track sandbox provision phases (ImagePull → ContainerCreate → Ready) |
| `jobs/` | Job handler implementations (sandbox, exec, batch, ssh, workflow) |
| `tee/` | TEE backends, attestation, sealed secrets (see TEE Architecture below) |

## Feature Map

### Sandbox Lifecycle

- Create / stop / resume / delete sidecar containers (local Docker)
- Multi-tier resume: hot (docker start) → warm (from committed image) → cold (from S3 snapshot)
- Snapshot via sidecar `/terminals/commands` (tar + curl upload to S3/HTTP destination)
- Auto-commit container filesystem on stop (`SANDBOX_SNAPSHOT_AUTO_COMMIT`)

Jobs:
- `JOB_SANDBOX_CREATE` (0)
- `JOB_SANDBOX_STOP` (1)
- `JOB_SANDBOX_RESUME` (2)
- `JOB_SANDBOX_DELETE` (3)
- `JOB_SANDBOX_SNAPSHOT` (4)

### Sidecar Execution

- `/terminals/commands` shell command execution
- `/agents/run` prompt (single turn)
- `/agents/run` task (multi-turn with session continuity)

All execution jobs update `last_activity_at` via `touch_sandbox()` to track idle time.

Jobs:
- `JOB_EXEC` (10)
- `JOB_PROMPT` (11)
- `JOB_TASK` (12)

### Batch Operations

- Create N sidecars locally (up to `MAX_BATCH_COUNT` = 50)
- Run task/exec across sidecar URLs (sequential or parallel)
- Collect in-memory batch results

Jobs:
- `JOB_BATCH_CREATE` (20)
- `JOB_BATCH_TASK` (21)
- `JOB_BATCH_EXEC` (22)
- `JOB_BATCH_COLLECT` (23)

### Workflows

- Store workflow configs on-chain when `JOB_WORKFLOW_CREATE` results are submitted
- Operators rebuild schedules on startup from on-chain registry (`bootstrap_workflows_from_chain`)
- Cron tick executes due workflows locally

Jobs:
- `JOB_WORKFLOW_CREATE` (30)
- `JOB_WORKFLOW_TRIGGER` (31)
- `JOB_WORKFLOW_CANCEL` (32)
- `JOB_WORKFLOW_TICK` (33) (internal scheduler)

### SSH Access

- Manage authorized_keys via sidecar `/terminals/commands`

Jobs:
- `JOB_SSH_PROVISION` (40)
- `JOB_SSH_REVOKE` (41)

## TEE Architecture

When `TEE_BACKEND` is set at startup, the operator initializes a TEE backend and sandboxes with
`tee_required: true` are deployed inside trusted execution environments instead of plain Docker.
The TEE integration is fully optional — without `TEE_BACKEND`, the blueprint operates exactly as
before.

### TEE Module Structure

```
tee/
├── mod.rs               TeeBackend trait, TeeConfig, TeeType, AttestationReport, shared helpers
├── backend_factory.rs   Runtime backend selection via TEE_BACKEND env var
├── sealed_secrets.rs    TeePublicKey, SealedSecret, SealedSecretResult types
├── sealed_secrets_api.rs  Operator API endpoints for public key + sealed secret injection
├── phala.rs             Phala dstack backend (TDX CVMs via dstack API)
├── aws_nitro.rs         AWS Nitro Enclaves backend (EC2 + Nitro enclave)
├── gcp.rs               GCP Confidential Space backend (Confidential VMs)
├── azure.rs             Azure Confidential VM + SKR backend (DCasv5 VMs)
└── direct.rs            Operator-managed TEE hardware (local Docker with attestation proxy)
```

Each backend is feature-gated: `tee-phala`, `tee-aws-nitro`, `tee-gcp`, `tee-azure`, `tee-direct`.
Use `tee-all` to enable all backends.

### TeeBackend Trait

All backends implement the async `TeeBackend` trait:

```rust
trait TeeBackend: Send + Sync {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment>;
    async fn attestation(&self, deployment_id: &str) -> Result<AttestationReport>;
    async fn stop(&self, deployment_id: &str) -> Result<()>;
    async fn destroy(&self, deployment_id: &str) -> Result<()>;
    fn tee_type(&self) -> TeeType;
    // Optional sealed secrets support:
    async fn derive_public_key(&self, deployment_id: &str) -> Result<TeePublicKey>;
    async fn inject_sealed_secrets(&self, deployment_id: &str, sealed: &SealedSecret) -> Result<SealedSecretResult>;
}
```

### 2-Phase Provisioning (TEE + Non-TEE)

Secret provisioning follows the same 2-phase pattern for both TEE and non-TEE sandboxes:

**Phase 1 — On-chain (`JOB_SANDBOX_CREATE`):**
```
Client → Tangle → Operator
                     │
                     ├─ tee_required=false → Docker container
                     └─ tee_required=true  → TEE deployment (Phala CVM, AWS Nitro, GCP, Azure, etc.)
                     │
                     ▼
              SandboxCreateOutput {
                  sandboxId, json: {
                      sandboxId, sidecarUrl, token, sshPort,
                      teeAttestationJson,   // empty if non-TEE
                      teePublicKeyJson      // empty if non-TEE
                  }
              }
```

**Phase 2 — Off-chain (operator API):**

For non-TEE sandboxes:
```
Client → POST /api/sandboxes/{id}/secrets → operator recreates container with merged env vars
```

For TEE sandboxes:
```
Client:
  1. Verify teeAttestationJson (enclave measurement matches expected code)
  2. Encrypt secrets to the TEE public key from teePublicKeyJson
  3. POST /api/sandboxes/{id}/tee/sealed-secrets → operator forwards opaque blob to TEE
  4. Only the TEE can decrypt inside the enclave
```

### Sealed Secrets API

Registered conditionally when a TEE backend is configured (`operator_api_router_with_tee`):

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/api/sandboxes/{id}/tee/public-key` | Session (EIP-191) | Fetch TEE-bound public key with attestation |
| `POST` | `/api/sandboxes/{id}/tee/sealed-secrets` | Session (EIP-191) | Inject encrypted secrets (operator cannot decrypt) |

### Supported Backends

| Backend | `TEE_BACKEND` value | TEE Type | Feature | Description |
|---------|---------------------|----------|---------|-------------|
| Phala dstack | `phala` | TDX | `tee-phala` | Intel TDX CVMs via Phala dstack API |
| AWS Nitro | `nitro` / `aws` | Nitro | `tee-aws-nitro` | EC2 instances with Nitro Enclaves |
| GCP Confidential Space | `gcp` | Sev | `tee-gcp` | AMD SEV-SNP Confidential VMs |
| Azure SKR | `azure` | Sev | `tee-azure` | DCasv5 Confidential VMs + Secure Key Release |
| Direct | `direct` | (configurable) | `tee-direct` | Operator-managed hardware with local attestation |

## Sandbox State Machine

```
                    create
                      │
                      ▼
                  ┌────────┐    idle timeout / max lifetime
                  │Running │ ──────────────────────────────┐
                  └───┬────┘                               │
                      │ stop                               │
                      ▼                                    ▼
                  ┌────────┐                          ┌────────┐
                  │Stopped │                          │Stopped │
                  │ (hot)  │                          │ (hot)  │
                  └───┬────┘                          └───┬────┘
                      │ GC hot→warm                       │
                      │ (docker commit + rm container)    │
                      ▼                                   │
                  ┌────────┐                              │
                  │ Warm   │ ◄────────────────────────────┘
                  │(image) │   (auto-commit if enabled)
                  └───┬────┘
                      │ GC warm→cold
                      │ (S3 upload + rm image)
                      ▼
                  ┌────────┐
                  │  Cold  │
                  │  (S3)  │
                  └───┬────┘
                      │ GC cold→gone
                      │ (delete S3 if operator-managed)
                      ▼
                  ┌────────┐
                  │  Gone  │ → record removed
                  └────────┘
```

Resume works from any tier:
- **Hot**: `docker start` the stopped container
- **Warm**: Create new container from committed snapshot image
- **Cold**: Create fresh container from base image, restore workspace from S3

## Tiered Garbage Collection

The reaper and GC run as background tasks:

- **Reaper** (`reaper_tick`, every `SANDBOX_REAPER_INTERVAL` seconds): enforces idle timeout and
  max lifetime on running sandboxes. Stops idle containers (with optional pre-stop S3 snapshot).
  Hard-deletes containers that exceed max lifetime.

- **GC** (`gc_tick`, every `SANDBOX_GC_INTERVAL` seconds): progressively demotes stopped sandboxes
  through storage tiers based on retention periods.

| Transition | Retention | Action |
|------------|-----------|--------|
| Hot → Warm | `SANDBOX_GC_HOT_RETENTION` (1 day) | `docker commit` + remove container |
| Warm → Cold | `SANDBOX_GC_WARM_RETENTION` (2 days) | Upload S3 snapshot + remove image |
| Cold → Gone | `SANDBOX_GC_COLD_RETENTION` (7 days) | Delete S3 object (operator-managed only) + remove record |

User-provided S3 destinations (BYOS3 via `snapshot_destination` on the record) are never deleted
by GC. The `is_operator_s3()` check compares the snapshot URL against
`SANDBOX_SNAPSHOT_DESTINATION_PREFIX` to distinguish operator-managed from user-managed snapshots.

## Startup Reconciliation

On startup, `reconcile_on_startup()` syncs the persistent store with Docker reality:
- Records pointing to missing containers are cleaned up
- Running containers not in the store are left alone (may belong to other services)

## Sidecar Auth Model

- Each sandbox gets a unique bearer token (cryptographically random, 32 bytes hex).
- Token is returned in `JOB_SANDBOX_CREATE` response.
- All sidecar jobs require the matching `sidecar_token`.
- Token comparison uses constant-time equality (`subtle::ConstantTimeEq`) to prevent timing attacks.

## On-Chain Workflow Registry

The blueprint contract stores workflow configs when `JOB_WORKFLOW_CREATE` results are submitted.
Operators rebuild schedules on startup by reading the registry:
- `getWorkflowIds(bool activeOnly)`
- `getWorkflow(uint64 workflowId)`

Task spec expected in `workflow_json`:

```json
{
  "sidecar_url": "https://operator.example/sidecar",
  "prompt": "run daily report",
  "session_id": "optional",
  "max_turns": 4,
  "model": "optional",
  "context_json": "{...}",
  "timeout_ms": 60000,
  "sidecar_token": "required"
}
```

## Metrics

The `metrics` module tracks operational counters via `AtomicU64` for on-chain QoS reporting:

- **Jobs**: `total_jobs`, `total_duration_ms`, `failed_jobs`, `total_input_tokens`, `total_output_tokens`
- **Sandboxes**: `active_sandboxes`, `peak_sandboxes`, `allocated_cpu_cores`, `allocated_memory_mb`
- **Sessions**: `active_sessions` (RAII guard prevents leaks)
- **Lifecycle**: `reaped_idle`, `reaped_lifetime`, `garbage_collected`
- **Snapshots**: `snapshots_committed`, `snapshots_uploaded`, `gc_containers_removed`, `gc_images_removed`, `gc_s3_cleaned`

When the optional `qos` feature is enabled, the binary periodically snapshots these counters and
submits them on-chain via `blueprint-qos`.

## Operator Selection

Operator selection is validated on-chain and can be deterministic. Clients should call
`previewOperatorSelection(count, seed)` and pass the selected operators plus the encoded
`SelectionRequest` in `requestInputs`.

```solidity
struct SelectionRequest {
    uint32 operatorCount;
    bytes32 seed;
    bool enforceDeterministic;
}
```

Batch jobs require results from all operators; other jobs accept a single result.

The on-chain contract also tracks per-operator capacity (`operatorMaxCapacity`,
`operatorActiveSandboxes`) for capacity-weighted assignment.

## Job Argument Schemas

```solidity
struct SandboxCreateRequest {
    string name;
    string image;
    string stack;
    string agent_identifier;
    string env_json;
    string metadata_json;
    bool ssh_enabled;
    string ssh_public_key;
    bool web_terminal_enabled;
    uint64 max_lifetime_seconds;
    uint64 idle_timeout_seconds;
    uint64 cpu_cores;
    uint64 memory_mb;
    uint64 disk_gb;
    bool tee_required;              // deploy inside TEE when true
    uint8 tee_type;                 // 0=None (operator chooses), 1=Tdx, 2=Nitro, 3=Sev
}

struct SandboxCreateOutput {
    string sandboxId;               // used by contract for sandbox→operator mapping
    string json;                    // full JSON response (includes teeAttestationJson, teePublicKeyJson if TEE)
}

struct SandboxIdRequest {
    string sandbox_id;
}

// Auth note: sidecar tokens are stored server-side and looked up from the sandbox record.
// They never appear in on-chain calldata. Secrets are injected via the operator API.

struct SandboxSnapshotRequest {
    string sidecar_url;
    string destination;
    bool include_workspace;
    bool include_state;
}

struct SandboxExecRequest {
    string sidecar_url;
    string command;
    string cwd;
    string env_json;
    uint64 timeout_ms;
}

struct SandboxPromptRequest {
    string sidecar_url;
    string message;
    string session_id;
    string model;
    string context_json;
    uint64 timeout_ms;
}

struct SandboxTaskRequest {
    string sidecar_url;
    string prompt;
    string session_id;
    uint64 max_turns;
    string model;
    string context_json;
    uint64 timeout_ms;
}

struct BatchCreateRequest {
    uint32 count;
    SandboxCreateRequest template_request;
    address[] operators;
    string distribution;
}

struct BatchTaskRequest {
    string[] sidecar_urls;
    string prompt;
    string session_id;
    uint64 max_turns;
    string model;
    string context_json;
    uint64 timeout_ms;
    bool parallel;
    string aggregation;
}

struct BatchExecRequest {
    string[] sidecar_urls;
    string command;
    string cwd;
    string env_json;
    uint64 timeout_ms;
    bool parallel;
}

struct BatchCollectRequest {
    string batch_id;
}

struct WorkflowCreateRequest {
    string name;
    string workflow_json;
    string trigger_type;      // "manual" | "cron"
    string trigger_config;    // cron expression
    string sandbox_config_json;
}

struct WorkflowControlRequest {
    uint64 workflow_id;
}

struct SshProvisionRequest {
    string sidecar_url;
    string username;
    string public_key;
}

struct SshRevokeRequest {
    string sidecar_url;
    string username;
    string public_key;
}
```

## Runtime Configuration

### Core

| Variable | Default | Description |
|----------|---------|-------------|
| `SIDECAR_IMAGE` | `ghcr.io/tangle-network/sidecar:latest` | Docker image for sidecar containers |
| `SIDECAR_PUBLIC_HOST` | `127.0.0.1` | Hostname for constructing sidecar URLs |
| `SIDECAR_HTTP_PORT` | `8080` | Container-internal HTTP port |
| `SIDECAR_SSH_PORT` | `22` | Container-internal SSH port |
| `SIDECAR_PULL_IMAGE` | `true` | Pull image on first use |
| `DOCKER_HOST` | (system default) | Docker daemon socket override |
| `REQUEST_TIMEOUT_SECS` | `30` | HTTP request timeout for sidecar calls |

### Sandbox Limits

| Variable | Default | Description |
|----------|---------|-------------|
| `SANDBOX_DEFAULT_IDLE_TIMEOUT` | `1800` (30m) | Default idle timeout when request specifies 0 |
| `SANDBOX_DEFAULT_MAX_LIFETIME` | `86400` (1d) | Default max lifetime when request specifies 0 |
| `SANDBOX_MAX_IDLE_TIMEOUT` | `7200` (2h) | Operator-enforced cap on idle timeout |
| `SANDBOX_MAX_MAX_LIFETIME` | `172800` (2d) | Operator-enforced cap on max lifetime |

### Reaper and GC

| Variable | Default | Description |
|----------|---------|-------------|
| `SANDBOX_REAPER_INTERVAL` | `30` | Seconds between reaper ticks |
| `SANDBOX_GC_INTERVAL` | `3600` (1h) | Seconds between GC ticks |
| `SANDBOX_GC_HOT_RETENTION` | `86400` (1d) | Keep stopped container before committing to image |
| `SANDBOX_GC_WARM_RETENTION` | `172800` (2d) | Keep committed image before uploading to S3 |
| `SANDBOX_GC_COLD_RETENTION` | `604800` (7d) | Keep S3 snapshot before final cleanup |

### Snapshots

| Variable | Default | Description |
|----------|---------|-------------|
| `SANDBOX_SNAPSHOT_AUTO_COMMIT` | `true` | Docker-commit container on stop |
| `SANDBOX_SNAPSHOT_DESTINATION_PREFIX` | (none) | Operator S3 prefix for managed snapshots |

### Workflows

| Variable | Default | Description |
|----------|---------|-------------|
| `WORKFLOW_CRON_SCHEDULE` | `0 * * * * *` | Cron expression for workflow tick frequency |

### QoS (optional, requires `qos` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `QOS_ENABLED` | `false` | Enable QoS metrics + heartbeat |
| `QOS_METRICS_INTERVAL_SECS` | (framework default) | Metrics collection interval |
| `QOS_DRY_RUN` | `false` | Log metrics without submitting on-chain |
| `SERVICE_ID` | (required) | Tangle service ID for heartbeat |
| `BLUEPRINT_ID` | (required) | Blueprint ID for heartbeat |
| `OPERATOR_MAX_CAPACITY` | (none) | Advertised max sandbox capacity (registration) |

### TEE (optional, requires TEE backend feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `TEE_BACKEND` | (none) | Backend to use: `phala`, `nitro`/`aws`, `gcp`, `azure`, `direct` |

#### Phala (`tee-phala` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `PHALA_API_KEY` | (required) | Phala dstack API key |
| `PHALA_API_ENDPOINT` | (Phala default) | Custom dstack API endpoint |

#### AWS Nitro (`tee-aws-nitro` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `AWS_REGION` | (required) | AWS region for Nitro Enclave instances |
| `AWS_NITRO_SUBNET_ID` | (required) | VPC subnet for enclave instances |
| `AWS_NITRO_SECURITY_GROUP_ID` | (required) | Security group allowing sidecar traffic |
| `AWS_NITRO_AMI_ID` | (required) | AMI with Nitro Enclave support |
| `AWS_NITRO_INSTANCE_TYPE` | `c5.xlarge` | EC2 instance type (must support enclaves) |
| `AWS_NITRO_KMS_KEY_ID` | (none) | KMS key for enclave-bound key policy |
| `AWS_NITRO_IAM_INSTANCE_PROFILE` | (none) | IAM profile for EC2 instances |

#### GCP Confidential Space (`tee-gcp` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `GCP_PROJECT_ID` | (required) | GCP project for Confidential VMs |
| `GCP_ZONE` | (required) | Compute zone for VM placement |
| `GCP_CONFIDENTIAL_SPACE_IMAGE` | (required) | Confidential Space base image |
| `GCP_MACHINE_TYPE` | `n2d-standard-4` | AMD SEV-SNP capable machine type |
| `GCP_SERVICE_ACCOUNT_EMAIL` | (none) | Service account for VMs |
| `GCP_NETWORK` | (none) | VPC network |
| `GCP_SUBNET` | (none) | VPC subnet |
| `GCP_KMS_KEY_RESOURCE` | (none) | Cloud KMS key for sealed secrets |

#### Azure SKR (`tee-azure` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `AZURE_SUBSCRIPTION_ID` | (required) | Azure subscription |
| `AZURE_RESOURCE_GROUP` | (required) | Resource group for Confidential VMs |
| `AZURE_LOCATION` | (required) | Azure region |
| `AZURE_VM_IMAGE` | (required) | VM image URN or ID |
| `AZURE_VM_SIZE` | `Standard_DC4as_v5` | DCasv5-series VM size |
| `AZURE_SUBNET_ID` | (required) | VNet subnet resource ID |
| `AZURE_KEY_VAULT_URL` | (none) | Key Vault URL for Secure Key Release |
| `AZURE_MAA_ENDPOINT` | (none) | Microsoft Azure Attestation endpoint |
| `AZURE_TENANT_ID` | (required) | Azure AD tenant |
| `AZURE_CLIENT_ID` | (required) | Service principal client ID |
| `AZURE_CLIENT_SECRET` | (required) | Service principal secret |

#### Direct (`tee-direct` feature)

| Variable | Default | Description |
|----------|---------|-------------|
| `TEE_DIRECT_TYPE` | (required) | Hardware TEE type: `tdx`, `sev`, or `nitro` |

## Output Model

- Job outputs are returned off-chain via the blueprint runtime.
- On-chain state is limited to workflow registry, sandbox→operator mapping, and operator capacity.
- Metrics are optionally reported on-chain via the QoS subsystem.

## Test Infrastructure

Tests are organized into five suites with different infrastructure requirements:

| Suite | Gate | Infrastructure | Count |
|-------|------|---------------|-------|
| `sidecar_integration` | (none) | Pure unit tests, mocked sidecar | ~80 |
| `integration` | (none) | Wiremock + optional Docker | ~52 |
| `real_sidecar` | `REAL_SIDECAR=1` | Docker + real sidecar container | ~46 |
| `snapshot_integration` | `SNAPSHOT_TEST=1` | Docker + MinIO (via `docker-compose.test.yml`) | ~9 |
| `anvil` | `SIDECAR_E2E=1` | Anvil blockchain + Docker | 1 |

MinIO test infrastructure is defined in `docker-compose.test.yml` (ports 9100/9101, auto-creates
`snapshots` bucket).
