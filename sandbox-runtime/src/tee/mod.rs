//! TEE (Trusted Execution Environment) types and backend trait.
//!
//! This module defines the configuration, attestation types, and the async
//! `TeeBackend` trait used to deploy sandboxes inside trusted execution
//! environments. Backend implementations live in feature-gated submodules.

#[cfg(feature = "tee-phala")]
pub mod phala;

#[cfg(feature = "tee-direct")]
pub mod direct;

#[cfg(feature = "tee-aws-nitro")]
pub mod aws_nitro;

#[cfg(feature = "tee-gcp")]
pub mod gcp;

#[cfg(feature = "tee-azure")]
pub mod azure;

pub mod backend_factory;
pub mod sealed_secrets;
pub mod sealed_secrets_api;

/// Supported TEE backend types.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TeeType {
    /// No TEE — standard Docker container (default).
    #[default]
    None,
    /// Intel TDX — VM-level isolation (Phala dstack, GCP C3, Azure DCesv5).
    Tdx,
    /// AWS Nitro Enclaves.
    Nitro,
    /// AMD SEV-SNP confidential VMs (Azure DCasv5, GCP N2D).
    Sev,
}

/// TEE configuration for sandbox creation.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TeeConfig {
    /// Whether TEE execution is required. If true and the operator cannot
    /// provide TEE, sandbox creation fails.
    pub required: bool,
    /// Preferred TEE backend. If `None` (default), the operator chooses.
    pub tee_type: TeeType,
}

/// Attestation report produced by a TEE runtime.
///
/// Returned to the customer so they can verify the sandbox is running
/// inside a genuine enclave with the expected code measurement.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AttestationReport {
    /// The TEE backend that produced this report.
    pub tee_type: TeeType,
    /// Raw attestation evidence (TDX report, Nitro attestation document, etc.).
    pub evidence: Vec<u8>,
    /// Enclave measurement (MRTD for TDX, PCR values for Nitro, etc.).
    pub measurement: Vec<u8>,
    /// Unix timestamp when the attestation was generated.
    pub timestamp: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// TeeBackend trait
// ─────────────────────────────────────────────────────────────────────────────

/// Parameters for deploying a container inside a TEE.
///
/// Constructed from `CreateSandboxParams` — see `TeeDeployParams::from_sandbox_params`.
#[derive(Clone, Debug)]
pub struct TeeDeployParams {
    pub sandbox_id: String,
    pub image: String,
    pub env_vars: Vec<(String, String)>,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    pub http_port: u16,
    pub ssh_port: Option<u16>,
    pub sidecar_token: String,
}

impl TeeDeployParams {
    /// Build TEE deploy params from a sandbox creation request.
    pub fn from_sandbox_params(
        sandbox_id: &str,
        params: &crate::runtime::CreateSandboxParams,
        container_port: u16,
        ssh_port: u16,
        token: &str,
    ) -> Self {
        let mut env_vars = vec![
            ("SIDECAR_PORT".to_string(), container_port.to_string()),
            (
                "SIDECAR_AUTH_TOKEN".to_string(),
                token.to_string(),
            ),
        ];

        // Parse env_json into env var pairs.
        if !params.env_json.trim().is_empty() {
            if let Ok(Some(serde_json::Value::Object(map))) =
                crate::util::parse_json_object(&params.env_json, "env_json")
            {
                for (key, value) in map {
                    let val = match value {
                        serde_json::Value::String(v) => v,
                        serde_json::Value::Number(v) => v.to_string(),
                        serde_json::Value::Bool(v) => v.to_string(),
                        _ => continue,
                    };
                    env_vars.push((key, val));
                }
            }
        }

        Self {
            sandbox_id: sandbox_id.to_string(),
            image: params.image.clone(),
            env_vars,
            cpu_cores: params.cpu_cores,
            memory_mb: params.memory_mb,
            disk_gb: params.disk_gb,
            http_port: container_port,
            ssh_port: if params.ssh_enabled {
                Some(ssh_port)
            } else {
                None
            },
            sidecar_token: token.to_string(),
        }
    }
}

/// Result of a successful TEE deployment.
#[derive(Clone, Debug)]
pub struct TeeDeployment {
    /// Backend-specific deployment ID (e.g. Phala app_id).
    pub deployment_id: String,
    /// Reachable URL for the sidecar HTTP API.
    pub sidecar_url: String,
    /// Host-mapped SSH port, if SSH was requested.
    pub ssh_port: Option<u16>,
    /// Attestation report from the TEE.
    pub attestation: AttestationReport,
    /// Opaque backend state, stored in SandboxRecord for later lifecycle ops.
    pub metadata_json: String,
}

/// Async trait for TEE backend implementations.
///
/// Each backend (Phala dstack, operator-managed TDX/SEV hardware, cloud TEE, etc.)
/// implements this trait to handle the full lifecycle of a TEE deployment.
#[async_trait::async_trait]
pub trait TeeBackend: Send + Sync {
    /// Deploy a container inside a TEE.
    async fn deploy(&self, params: &TeeDeployParams) -> crate::error::Result<TeeDeployment>;

    /// Retrieve fresh attestation for a running deployment.
    async fn attestation(&self, deployment_id: &str) -> crate::error::Result<AttestationReport>;

    /// Stop a TEE deployment (may be resumable depending on backend).
    async fn stop(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Destroy a TEE deployment and clean up all resources.
    async fn destroy(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Which TEE type this backend provides.
    fn tee_type(&self) -> TeeType;

    // ── Sealed secrets (optional, default: not supported) ────────────────

    /// Derive a TEE-bound public key for sealed secret encryption.
    ///
    /// The returned key is bound to the enclave measurement via attestation.
    /// Clients verify the attestation before encrypting secrets to this key.
    ///
    /// Default: returns an error indicating sealed secrets are not supported.
    async fn derive_public_key(
        &self,
        deployment_id: &str,
    ) -> crate::error::Result<sealed_secrets::TeePublicKey> {
        let _ = deployment_id;
        Err(crate::error::SandboxError::Validation(format!(
            "Sealed secrets not supported by {:?} backend",
            self.tee_type()
        )))
    }

    /// Inject sealed (encrypted) secrets into a TEE deployment.
    ///
    /// The operator calls this to forward the client's encrypted blob to the
    /// sidecar running inside the TEE. Only the TEE can decrypt.
    ///
    /// Default: returns an error indicating sealed secrets are not supported.
    async fn inject_sealed_secrets(
        &self,
        deployment_id: &str,
        sealed: &sealed_secrets::SealedSecret,
    ) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
        let _ = (deployment_id, sealed);
        Err(crate::error::SandboxError::Validation(format!(
            "Sealed secrets not supported by {:?} backend",
            self.tee_type()
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers for cloud TEE backends
// ─────────────────────────────────────────────────────────────────────────────

/// Look up the sidecar URL and auth token for a TEE deployment by its deployment ID.
///
/// Scans the sandbox store for a record whose `tee_deployment_id` matches.
pub(crate) fn sidecar_info_for_deployment(
    deployment_id: &str,
) -> crate::error::Result<(String, String)> {
    let store = crate::runtime::sandboxes()?;
    let record = store
        .find(|r| r.tee_deployment_id.as_deref() == Some(deployment_id))?
        .ok_or_else(|| {
            crate::error::SandboxError::NotFound(format!(
                "No sandbox found for TEE deployment '{deployment_id}'"
            ))
        })?;
    Ok((record.sidecar_url.clone(), record.token.clone()))
}

/// Fetch fresh attestation from a running sidecar's `/tee/attestation` endpoint.
pub(crate) async fn fetch_sidecar_attestation(
    sidecar_url: &str,
    token: &str,
) -> crate::error::Result<AttestationReport> {
    let url = crate::http::build_url(sidecar_url, "/tee/attestation")?;
    let headers = crate::http::auth_headers(token)?;
    let (_status, body) =
        crate::http::send_json(reqwest::Method::GET, url, None, headers).await?;
    serde_json::from_str(&body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid attestation response: {e}"))
    })
}

/// Poll a sidecar's `/health` endpoint until it responds successfully.
pub(crate) async fn wait_for_sidecar_health(
    sidecar_url: &str,
    token: &str,
    timeout: std::time::Duration,
) -> crate::error::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(crate::error::SandboxError::CloudProvider(
                "Sidecar health check timed out".into(),
            ));
        }
        if let (Ok(url), Ok(headers)) = (
            crate::http::build_url(sidecar_url, "/health"),
            crate::http::auth_headers(token),
        ) {
            if crate::http::send_json(reqwest::Method::GET, url, None, headers)
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Derive a TEE-bound public key by proxying to the sidecar.
pub(crate) async fn sidecar_derive_public_key(
    deployment_id: &str,
) -> crate::error::Result<sealed_secrets::TeePublicKey> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let url = crate::http::build_url(&sidecar_url, "/tee/public-key")?;
    let headers = crate::http::auth_headers(&token)?;
    let (_status, body) =
        crate::http::send_json(reqwest::Method::GET, url, None, headers).await?;
    serde_json::from_str(&body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid TeePublicKey response: {e}"))
    })
}

/// Inject sealed secrets by proxying to the sidecar.
pub(crate) async fn sidecar_inject_sealed_secrets(
    deployment_id: &str,
    sealed: &sealed_secrets::SealedSecret,
) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let payload = serde_json::to_value(sealed).map_err(|e| {
        crate::error::SandboxError::Validation(format!(
            "Failed to serialize sealed secret: {e}"
        ))
    })?;
    let resp =
        crate::http::sidecar_post_json(&sidecar_url, "/tee/sealed-secrets", &token, payload)
            .await?;
    serde_json::from_value(resp).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid SealedSecretResult response: {e}"))
    })
}
