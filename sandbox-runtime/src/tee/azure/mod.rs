//! Azure Confidential VM + Secure Key Release (SKR) TEE backend.
//!
//! Deploys sidecar containers on Azure Confidential VMs (DCasv5/ECasv5)
//! running AMD SEV-SNP. Uses Microsoft Azure Attestation (MAA) for
//! hardware attestation validation and Key Vault SKR for secret release.
//!
//! # Deploy flow
//!
//! 1. Create a public IP and NIC in the configured subnet.
//! 2. Create a Confidential VM (DCasv5 or ECasv5 series, SEV-SNP) with the
//!    sidecar pre-installed in the VM image. System-assigned managed identity
//!    is enabled for Key Vault access.
//! 3. The sidecar reads the SEV-SNP attestation report from the vTPM NV Index
//!    (`0x01400001`), sends it to MAA for validation, and receives a signed JWT.
//! 4. Key Vault SKR validates the MAA JWT and releases the wrapped key
//!    to the TEE using the ephemeral `TpmEphemeralEncryptionKey`.
//!
//! # Sealed secrets
//!
//! The HCL (Host Compatibility Layer) generates an ephemeral RSA key pair at
//! boot, seals the private key to the vTPM. MAA embeds the public key in
//! `x-ms-runtime.keys`. Key Vault's `/release` endpoint validates the MAA
//! JWT, wraps the secret to the TEE's ephemeral key. Only the TEE holding
//! the vTPM-sealed private key can unwrap.
//!
//! # Authentication
//!
//! Uses OAuth2 client credentials flow with `AZURE_TENANT_ID`,
//! `AZURE_CLIENT_ID`, and `AZURE_CLIENT_SECRET`. This avoids a dependency
//! on `azure_identity` while supporting service principal auth.

use std::time::Duration;

use tokio::sync::RwLock;

use base64::Engine;

use super::sealed_secrets::{SealedSecret, SealedSecretResult, TeePublicKey};
use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};

const COMPUTE_API_VERSION: &str = "2024-07-01";
const NETWORK_API_VERSION: &str = "2023-11-01";

/// Configuration for the Azure SKR backend, read from environment variables.
#[derive(Clone, Debug)]
pub struct AzureConfig {
    pub subscription_id: String,
    pub resource_group: String,
    pub location: String,
    pub vm_image: String,
    pub vm_size: String,
    pub subnet_id: String,
    pub key_vault_url: Option<String>,
    pub maa_endpoint: Option<String>,
    // OAuth2 client credentials
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
}

impl AzureConfig {
    /// Load configuration from environment variables.
    ///
    /// Required: `AZURE_SUBSCRIPTION_ID`, `AZURE_RESOURCE_GROUP`, `AZURE_LOCATION`,
    /// `AZURE_VM_IMAGE`, `AZURE_SUBNET_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
    /// `AZURE_CLIENT_SECRET`.
    /// Optional: `AZURE_VM_SIZE` (default: Standard_DC4as_v5),
    /// `AZURE_KEY_VAULT_URL`, `AZURE_MAA_ENDPOINT`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            subscription_id: require_env("AZURE_SUBSCRIPTION_ID")?,
            resource_group: require_env("AZURE_RESOURCE_GROUP")?,
            location: require_env("AZURE_LOCATION")?,
            vm_image: require_env("AZURE_VM_IMAGE")?,
            vm_size: std::env::var("AZURE_VM_SIZE")
                .unwrap_or_else(|_| "Standard_DC4as_v5".to_string()),
            subnet_id: require_env("AZURE_SUBNET_ID")?,
            key_vault_url: std::env::var("AZURE_KEY_VAULT_URL").ok(),
            maa_endpoint: std::env::var("AZURE_MAA_ENDPOINT").ok(),
            tenant_id: require_env("AZURE_TENANT_ID")?,
            client_id: require_env("AZURE_CLIENT_ID")?,
            client_secret: require_env("AZURE_CLIENT_SECRET")?,
        })
    }
}

/// Cached OAuth2 access token.
pub(crate) struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

/// TEE backend that deploys containers on Azure Confidential VMs with SKR.
pub struct AzureSkrBackend {
    pub config: AzureConfig,
    pub(crate) http: reqwest::Client,
    pub(crate) token_cache: RwLock<Option<CachedToken>>,
}

mod backend;
mod methods;

// tee-level helpers the moved impl code reaches via `super::` (azure is now a submodule).
pub(crate) use super::{
    fetch_sidecar_attestation, sidecar_derive_public_key, sidecar_info_for_deployment,
    sidecar_inject_sealed_secrets, wait_for_sidecar_health,
};

pub(crate) fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| {
        SandboxError::Validation(format!(
            "Azure SKR backend requires {name} environment variable"
        ))
    })
}
