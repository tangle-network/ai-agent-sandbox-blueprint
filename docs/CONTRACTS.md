# Layer Contracts

This document defines the minimal cross-layer contracts for the blueprint family.

## Non-Negotiable Rule

On-chain jobs are for state transitions only.

- If an operation mutates authoritative state: on-chain job.
- If an operation is read-only or operational I/O: `eth_call` and/or operator HTTP API.

## Current State (March 3, 2026)

- `sandbox-runtime` currently contains both runtime contracts and concrete Docker/TEE integrations.
- `microvm-blueprint` is the target L0 substrate and should be consumed only via L1 adapters.
- `ai-agent-tee-instance-blueprint-lib` currently depends on `ai-agent-instance-blueprint-lib` (same-product variant coupling). This is a temporary exception that should be removed by extracting shared instance runtime logic to L1.

## Contract 1: `SandboxProvider` (L0/L1 boundary)

Infra-facing lifecycle contract. Implemented by Docker/microVM/TEE providers.

```rust
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    async fn create(&self, req: CreateSandboxRequest) -> Result<CreateSandboxResult, ProviderError>;
    async fn stop(&self, sandbox_id: &str) -> Result<(), ProviderError>;
    async fn resume(&self, sandbox_id: &str) -> Result<ResumeResult, ProviderError>;
    async fn destroy(&self, sandbox_id: &str) -> Result<(), ProviderError>;
    async fn status(&self, sandbox_id: &str) -> Result<SandboxStatus, ProviderError>;
}
```

Data contract:

```rust
pub struct CreateSandboxRequest {
    pub sandbox_id: String,
    pub owner: String,
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    pub env: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
    pub tee: Option<TeeRequest>,
}

pub struct CreateSandboxResult {
    pub sandbox_id: String,
    pub sidecar_url: String,
    pub ssh_port: Option<u16>,
    pub attestation_json: Option<String>,
    pub tee_deployment_id: Option<String>,
}
```

## Contract 2: `RuntimeAdapter` (L1 stable surface)

Product-facing runtime contract. Products should consume this instead of provider internals.

```rust
#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn provision(&self, req: RuntimeProvisionRequest) -> Result<RuntimeProvisionResult, RuntimeError>;
    async fn deprovision(&self, req: RuntimeDeprovisionRequest) -> Result<(), RuntimeError>;
    async fn exec(&self, req: RuntimeExecRequest) -> Result<RuntimeExecResult, RuntimeError>;
    async fn prompt(&self, req: RuntimePromptRequest) -> Result<RuntimePromptResult, RuntimeError>;
    async fn task(&self, req: RuntimeTaskRequest) -> Result<RuntimeTaskResult, RuntimeError>;
}
```

Data contract:

```rust
pub struct RuntimeProvisionRequest {
    pub service_id: u64,
    pub owner: String,
    pub template_id: String,
    pub tenant: TenantProfile,
}

pub struct RuntimeProvisionResult {
    pub sandbox_id: String,
    pub sidecar_url: String,
    pub ssh_port: Option<u16>,
    pub tee_attestation_json: Option<String>,
    pub tee_public_key_json: Option<String>,
}
```

## Contract 3: `TemplatePack` (product -> runtime configuration bundle)

Versioned configuration pack for runtime provisioning and policy defaults.

```rust
pub trait TemplatePack: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn sidecar_image(&self) -> &str;
    fn env_defaults(&self) -> &BTreeMap<String, String>;
    fn runtime_limits(&self) -> RuntimeLimits;
}
```

Data contract:

```rust
pub struct RuntimeLimits {
    pub max_lifetime_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
}
```

## Contract 4: `TenantProfile` (authz + quota policy input)

Tenant-specific policy used by runtime and operator API enforcement.

```rust
pub struct TenantProfile {
    pub tenant_id: String,
    pub owner_address: String,
    pub permitted_callers: Vec<String>,
    pub rate_tier: RateTier,
    pub quota: TenantQuota,
    pub metadata: serde_json::Value,
}

pub struct TenantQuota {
    pub max_sandboxes: u32,
    pub max_concurrent_execs: u32,
    pub max_storage_gb: u32,
}
```

## Contract 5: `InstanceLifecycleReporter` (operator -> manager on-chain sync)

Direct instance lifecycle sync surface for operator-signed transactions.

```rust
pub trait InstanceLifecycleReporter {
    fn report_provisioned(
        service_id: u64,
        sandbox_id: String,
        sidecar_url: String,
        ssh_port: u32,
        tee_attestation_json: String,
    );

    fn report_deprovisioned(service_id: u64);
}
```

Validation rules:
- `msg.sender` must be an active service operator (`isServiceOperator`).
- `instanceMode` must be enabled on manager.
- TEE manager requires non-empty attestation on provision.
- Direct report is canonical for startup reconciliation.

## Dependency Rules

- Allowed: `Product (L2) -> RuntimeAdapter (L1) -> SandboxProvider (L0)`
- Forbidden: `L2 -> L0` direct dependencies
- Forbidden: cross-product dependencies (`L2 -> L2`)
- Temporary exception: tee-instance crate reusing instance crate internals inside this repo; remove by extracting shared logic into L1.

## Compatibility Policy

- Additive fields are allowed with safe defaults.
- Removing or changing field semantics requires a major version bump.
- Any contract-breaking change requires migration notes and rollout plan in PR.
