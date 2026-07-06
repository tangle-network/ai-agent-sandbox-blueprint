//! TEE (Trusted Execution Environment) types and backend trait.
//!
//! This module defines the configuration, attestation types, and the async
//! `TeeBackend` trait used to deploy sandboxes inside trusted execution
//! environments. Backend implementations live in feature-gated submodules.

#[cfg(feature = "tee-phala")]
pub mod phala;

#[cfg(feature = "tee-direct")]
pub mod attestation;

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

/// Real cryptographic quote verification (Intel TDX DCAP, AMD SEV-SNP, AWS
/// Nitro). Gated so the default and non-TEE builds never pull the heavier
/// X.509/ECDSA crates.
#[cfg(feature = "tee-verify")]
mod verify;

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
    /// Optional caller-supplied attestation nonce/report data.
    ///
    /// TDX and SEV-SNP reports take exactly 64 bytes of report data. Callers
    /// may supply 32-64 bytes; shorter values are right-padded with zeros.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_nonce: Option<Vec<u8>>,
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
    /// Extra container ports to expose (e.g. user web server on 3000).
    pub extra_ports: Vec<u16>,
    /// Optional caller-supplied report data for deploy-time attestation.
    pub attestation_report_data: Option<[u8; 64]>,
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
            ("SIDECAR_AUTH_TOKEN".to_string(), token.to_string()),
        ];

        if let Some(caps) = crate::runtime::parse_sidecar_capabilities(&params.capabilities_json) {
            env_vars.push(("SIDECAR_CAPABILITIES".to_string(), caps));
        }

        // Parse env_json into env var pairs.
        if !params.env_json.trim().is_empty()
            && let Ok(Some(serde_json::Value::Object(map))) =
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
            extra_ports: params.port_mappings.clone(),
            attestation_report_data: params
                .tee_config
                .as_ref()
                .and_then(|cfg| cfg.attestation_report_data()),
        }
    }
}

impl TeeConfig {
    /// Normalize caller-supplied nonce bytes into 64-byte report data.
    pub fn attestation_report_data(&self) -> Option<[u8; 64]> {
        match self.attestation_nonce.as_ref() {
            Some(nonce) => pad_attestation_nonce(nonce).ok().flatten(),
            None => None,
        }
    }

    /// Set attestation nonce bytes after validating length.
    pub fn with_attestation_nonce(mut self, nonce: Option<Vec<u8>>) -> crate::error::Result<Self> {
        if let Some(ref value) = nonce {
            validate_attestation_nonce(value)?;
        }
        self.attestation_nonce = nonce;
        Ok(self)
    }
}

/// Decode a hex-encoded caller nonce. Accepts optional `0x` prefix.
pub fn decode_attestation_nonce_hex(value: &str) -> crate::error::Result<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if !hex.len().is_multiple_of(2) {
        return Err(crate::error::SandboxError::Validation(
            "attestation_nonce must be even-length hex".into(),
        ));
    }
    let bytes = hex::decode(hex).map_err(|e| {
        crate::error::SandboxError::Validation(format!("attestation_nonce must be hex: {e}"))
    })?;
    validate_attestation_nonce(&bytes)?;
    Ok(bytes)
}

/// Validate caller nonce size. Empty means "not supplied".
pub fn validate_attestation_nonce(nonce: &[u8]) -> crate::error::Result<()> {
    if nonce.is_empty() {
        return Ok(());
    }
    if !(32..=64).contains(&nonce.len()) {
        return Err(crate::error::SandboxError::Validation(format!(
            "attestation_nonce must be 32-64 bytes, got {}",
            nonce.len()
        )));
    }
    Ok(())
}

/// Convert caller nonce bytes into fixed-size TEE report data.
pub fn pad_attestation_nonce(nonce: &[u8]) -> crate::error::Result<Option<[u8; 64]>> {
    validate_attestation_nonce(nonce)?;
    if nonce.is_empty() {
        return Ok(None);
    }
    let mut report_data = [0u8; 64];
    report_data[..nonce.len()].copy_from_slice(nonce);
    Ok(Some(report_data))
}

mod backend;
mod sidecar_attest;
mod verification;
mod verify_flow;

pub use backend::*;
pub(crate) use sidecar_attest::*;
pub use verification::*;
pub use verify_flow::*;

#[cfg(any(test, feature = "test-utils"))]
pub mod mock;

#[cfg(test)]
mod tests;
