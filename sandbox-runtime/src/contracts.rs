//! Layer contracts and adapter implementations for sandbox runtime (`L1`).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::SandboxError;
use crate::runtime::{
    CreateSandboxParams, SandboxState, create_sidecar, delete_sidecar, get_sandbox_by_id,
    resume_sidecar, stop_sidecar,
};
use crate::tee::{TeeBackend, TeeConfig};

pub type ProviderError = SandboxError;
pub type RuntimeError = SandboxError;

/// Infra-facing lifecycle contract. Implemented by Docker/microVM/TEE providers.
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    async fn create(
        &self,
        req: CreateSandboxRequest,
    ) -> std::result::Result<CreateSandboxResult, ProviderError>;
    async fn stop(&self, sandbox_id: &str) -> std::result::Result<(), ProviderError>;
    async fn resume(&self, sandbox_id: &str) -> std::result::Result<ResumeResult, ProviderError>;
    async fn destroy(&self, sandbox_id: &str) -> std::result::Result<(), ProviderError>;
    async fn status(&self, sandbox_id: &str) -> std::result::Result<SandboxStatus, ProviderError>;
}

#[derive(Clone, Debug)]
pub struct CreateSandboxRequest {
    /// Logical sandbox identifier requested by caller.
    /// Runtime may assign a concrete ID and return it in `CreateSandboxResult`.
    pub sandbox_id: String,
    pub owner: String,
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    pub env: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
    pub tee: Option<TeeConfig>,
}

#[derive(Clone, Debug)]
pub struct CreateSandboxResult {
    pub sandbox_id: String,
    pub sidecar_url: String,
    pub ssh_port: Option<u16>,
    pub attestation_json: Option<String>,
    pub tee_deployment_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResumeResult {
    pub sidecar_url: String,
    pub ssh_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxStatus {
    Running,
    Stopped,
    Missing,
}

/// Docker-backed provider using the existing sandbox-runtime container lifecycle.
#[derive(Clone, Default)]
pub struct DockerSandboxProvider {
    tee_backend: Option<Arc<dyn TeeBackend>>,
}

impl DockerSandboxProvider {
    pub fn new(tee_backend: Option<Arc<dyn TeeBackend>>) -> Self {
        Self { tee_backend }
    }
}

#[async_trait]
impl SandboxProvider for DockerSandboxProvider {
    async fn create(
        &self,
        req: CreateSandboxRequest,
    ) -> std::result::Result<CreateSandboxResult, ProviderError> {
        let env_json = serde_json::to_string(&req.env)
            .map_err(|e| SandboxError::Validation(format!("invalid env map: {e}")))?;
        let metadata_json = serde_json::json!({ "labels": req.labels }).to_string();
        let params = CreateSandboxParams {
            name: req.sandbox_id.clone(),
            image: req.image,
            stack: "default".to_string(),
            agent_identifier: req.sandbox_id,
            env_json,
            metadata_json,
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 0,
            idle_timeout_seconds: 0,
            cpu_cores: req.cpu_cores,
            memory_mb: req.memory_mb,
            disk_gb: req.disk_gb,
            owner: req.owner,
            service_id: None,
            tee_config: req.tee,
            user_env_json: "{}".to_string(),
            port_mappings: Vec::new(),
        };

        let (record, attestation) = create_sidecar(&params, self.tee_backend.as_deref()).await?;

        Ok(CreateSandboxResult {
            sandbox_id: record.id,
            sidecar_url: record.sidecar_url,
            ssh_port: record.ssh_port,
            attestation_json: attestation.and_then(|a| serde_json::to_string(&a).ok()),
            tee_deployment_id: record.tee_deployment_id,
        })
    }

    async fn stop(&self, sandbox_id: &str) -> std::result::Result<(), ProviderError> {
        let record = get_sandbox_by_id(sandbox_id)?;
        stop_sidecar(&record).await
    }

    async fn resume(&self, sandbox_id: &str) -> std::result::Result<ResumeResult, ProviderError> {
        let record = get_sandbox_by_id(sandbox_id)?;
        resume_sidecar(&record).await?;
        let updated = get_sandbox_by_id(sandbox_id)?;
        Ok(ResumeResult {
            sidecar_url: updated.sidecar_url,
            ssh_port: updated.ssh_port,
        })
    }

    async fn destroy(&self, sandbox_id: &str) -> std::result::Result<(), ProviderError> {
        let record = get_sandbox_by_id(sandbox_id)?;
        delete_sidecar(&record, self.tee_backend.as_deref()).await
    }

    async fn status(&self, sandbox_id: &str) -> std::result::Result<SandboxStatus, ProviderError> {
        match get_sandbox_by_id(sandbox_id) {
            Ok(record) => match record.state {
                SandboxState::Running => Ok(SandboxStatus::Running),
                SandboxState::Stopped => Ok(SandboxStatus::Stopped),
            },
            Err(SandboxError::NotFound(_)) => Ok(SandboxStatus::Missing),
            Err(e) => Err(e),
        }
    }
}

/// Product-facing runtime contract.
#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn provision(
        &self,
        req: RuntimeProvisionRequest,
    ) -> std::result::Result<RuntimeProvisionResult, RuntimeError>;
    async fn deprovision(
        &self,
        req: RuntimeDeprovisionRequest,
    ) -> std::result::Result<(), RuntimeError>;
    async fn exec(
        &self,
        req: RuntimeExecRequest,
    ) -> std::result::Result<RuntimeExecResult, RuntimeError>;
    async fn prompt(
        &self,
        req: RuntimePromptRequest,
    ) -> std::result::Result<RuntimePromptResult, RuntimeError>;
    async fn task(
        &self,
        req: RuntimeTaskRequest,
    ) -> std::result::Result<RuntimeTaskResult, RuntimeError>;
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeLimits {
    pub max_lifetime_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
}

pub trait TemplatePack: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn sidecar_image(&self) -> &str;
    fn env_defaults(&self) -> &BTreeMap<String, String>;
    fn runtime_limits(&self) -> RuntimeLimits;
}

#[derive(Clone, Debug)]
pub struct StaticTemplatePack {
    pub id: String,
    pub version: String,
    pub sidecar_image: String,
    pub env_defaults: BTreeMap<String, String>,
    pub runtime_limits: RuntimeLimits,
}

impl TemplatePack for StaticTemplatePack {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn sidecar_image(&self) -> &str {
        &self.sidecar_image
    }

    fn env_defaults(&self) -> &BTreeMap<String, String> {
        &self.env_defaults
    }

    fn runtime_limits(&self) -> RuntimeLimits {
        self.runtime_limits
    }
}

#[derive(Clone, Debug)]
pub enum RateTier {
    Basic,
    Pro,
    Enterprise,
}

#[derive(Clone, Debug)]
pub struct TenantQuota {
    pub max_sandboxes: u32,
    pub max_concurrent_execs: u32,
    pub max_storage_gb: u32,
}

#[derive(Clone, Debug)]
pub struct TenantProfile {
    pub tenant_id: String,
    pub owner_address: String,
    pub permitted_callers: Vec<String>,
    pub rate_tier: RateTier,
    pub quota: TenantQuota,
    pub metadata: Value,
}

#[derive(Clone, Debug)]
pub struct RuntimeProvisionRequest {
    pub service_id: u64,
    pub owner: String,
    pub template_id: String,
    pub tenant: TenantProfile,
    pub tee: Option<TeeConfig>,
}

#[derive(Clone, Debug)]
pub struct RuntimeProvisionResult {
    pub sandbox_id: String,
    pub sidecar_url: String,
    pub ssh_port: Option<u16>,
    pub tee_attestation_json: Option<String>,
    pub tee_public_key_json: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuntimeDeprovisionRequest {
    pub service_id: u64,
    pub sandbox_id: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeExecRequest {
    pub sandbox_id: String,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Clone, Debug)]
pub struct RuntimePromptRequest {
    pub sandbox_id: String,
    pub prompt: String,
}

#[derive(Clone, Debug)]
pub struct RuntimePromptResult {
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeTaskRequest {
    pub sandbox_id: String,
    pub task: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeTaskResult {
    pub status: String,
}

/// Default L1 adapter that maps product-facing requests onto a `SandboxProvider`.
pub struct DefaultRuntimeAdapter<P: SandboxProvider> {
    provider: P,
    templates: BTreeMap<String, StaticTemplatePack>,
}

impl<P: SandboxProvider> DefaultRuntimeAdapter<P> {
    pub fn new(provider: P, templates: BTreeMap<String, StaticTemplatePack>) -> Self {
        Self {
            provider,
            templates,
        }
    }

    pub fn with_default_template(provider: P, image: String) -> Self {
        let mut templates = BTreeMap::new();
        templates.insert(
            "default".to_string(),
            StaticTemplatePack {
                id: "default".to_string(),
                version: "1".to_string(),
                sidecar_image: image,
                env_defaults: BTreeMap::new(),
                runtime_limits: RuntimeLimits {
                    max_lifetime_seconds: 86_400,
                    idle_timeout_seconds: 1_800,
                    cpu_cores: 2,
                    memory_mb: 4_096,
                    disk_gb: 20,
                },
            },
        );
        Self::new(provider, templates)
    }

    fn template(&self, id: &str) -> std::result::Result<&StaticTemplatePack, RuntimeError> {
        self.templates
            .get(id)
            .ok_or_else(|| SandboxError::Validation(format!("unknown template_id '{id}'")))
    }
}

#[async_trait]
impl<P: SandboxProvider> RuntimeAdapter for DefaultRuntimeAdapter<P> {
    async fn provision(
        &self,
        req: RuntimeProvisionRequest,
    ) -> std::result::Result<RuntimeProvisionResult, RuntimeError> {
        let template = self.template(&req.template_id)?;
        let limits = template.runtime_limits();
        let mut labels = BTreeMap::new();
        labels.insert("service_id".to_string(), req.service_id.to_string());
        labels.insert("tenant_id".to_string(), req.tenant.tenant_id.clone());
        labels.insert("template_id".to_string(), req.template_id.clone());

        let created = self
            .provider
            .create(CreateSandboxRequest {
                sandbox_id: format!("svc-{}-{}", req.service_id, req.tenant.tenant_id),
                owner: req.owner,
                image: template.sidecar_image().to_string(),
                cpu_cores: limits.cpu_cores,
                memory_mb: limits.memory_mb,
                disk_gb: limits.disk_gb,
                env: template.env_defaults().clone(),
                labels,
                tee: req.tee,
            })
            .await?;

        Ok(RuntimeProvisionResult {
            sandbox_id: created.sandbox_id,
            sidecar_url: created.sidecar_url,
            ssh_port: created.ssh_port,
            tee_attestation_json: created.attestation_json,
            tee_public_key_json: None,
        })
    }

    async fn deprovision(
        &self,
        req: RuntimeDeprovisionRequest,
    ) -> std::result::Result<(), RuntimeError> {
        let _ = req.service_id;
        self.provider.destroy(&req.sandbox_id).await
    }

    async fn exec(
        &self,
        _req: RuntimeExecRequest,
    ) -> std::result::Result<RuntimeExecResult, RuntimeError> {
        Err(SandboxError::Validation(
            "runtime exec is served by operator HTTP API, not RuntimeAdapter".to_string(),
        ))
    }

    async fn prompt(
        &self,
        _req: RuntimePromptRequest,
    ) -> std::result::Result<RuntimePromptResult, RuntimeError> {
        Err(SandboxError::Validation(
            "runtime prompt is served by operator HTTP API, not RuntimeAdapter".to_string(),
        ))
    }

    async fn task(
        &self,
        _req: RuntimeTaskRequest,
    ) -> std::result::Result<RuntimeTaskResult, RuntimeError> {
        Err(SandboxError::Validation(
            "runtime task is served by operator HTTP API, not RuntimeAdapter".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockProvider {
        create_req: Mutex<Option<CreateSandboxRequest>>,
    }

    #[async_trait]
    impl SandboxProvider for MockProvider {
        async fn create(
            &self,
            req: CreateSandboxRequest,
        ) -> std::result::Result<CreateSandboxResult, ProviderError> {
            *self.create_req.lock().expect("lock create req") = Some(req);
            Ok(CreateSandboxResult {
                sandbox_id: "sb_1".to_string(),
                sidecar_url: "http://127.0.0.1:1234".to_string(),
                ssh_port: Some(2222),
                attestation_json: None,
                tee_deployment_id: None,
            })
        }

        async fn stop(&self, _sandbox_id: &str) -> std::result::Result<(), ProviderError> {
            Ok(())
        }

        async fn resume(
            &self,
            _sandbox_id: &str,
        ) -> std::result::Result<ResumeResult, ProviderError> {
            Ok(ResumeResult {
                sidecar_url: "http://127.0.0.1:1234".to_string(),
                ssh_port: Some(2222),
            })
        }

        async fn destroy(&self, _sandbox_id: &str) -> std::result::Result<(), ProviderError> {
            Ok(())
        }

        async fn status(
            &self,
            _sandbox_id: &str,
        ) -> std::result::Result<SandboxStatus, ProviderError> {
            Ok(SandboxStatus::Running)
        }
    }

    #[tokio::test]
    async fn runtime_adapter_maps_template_to_provider_create() {
        let provider = MockProvider::default();
        let adapter = DefaultRuntimeAdapter::with_default_template(
            provider,
            "ghcr.io/tangle/sidecar:dev".to_string(),
        );

        let result = adapter
            .provision(RuntimeProvisionRequest {
                service_id: 42,
                owner: "0xabc".to_string(),
                template_id: "default".to_string(),
                tenant: TenantProfile {
                    tenant_id: "tenant-a".to_string(),
                    owner_address: "0xabc".to_string(),
                    permitted_callers: vec![],
                    rate_tier: RateTier::Basic,
                    quota: TenantQuota {
                        max_sandboxes: 1,
                        max_concurrent_execs: 1,
                        max_storage_gb: 10,
                    },
                    metadata: Value::Null,
                },
                tee: None,
            })
            .await
            .expect("provision");

        assert_eq!(result.sandbox_id, "sb_1");
        assert_eq!(result.ssh_port, Some(2222));
        assert_eq!(result.sidecar_url, "http://127.0.0.1:1234");
    }
}
