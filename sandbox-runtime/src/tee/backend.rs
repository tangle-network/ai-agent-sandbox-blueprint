//! TEE deployment result, the async TeeBackend trait, and the process-wide backend registry.

use super::*;

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
    /// Extra port mappings: container_port → host_port.
    pub extra_ports: std::collections::HashMap<u16, u16>,
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
    async fn attestation(
        &self,
        deployment_id: &str,
        report_data: Option<[u8; 64]>,
    ) -> crate::error::Result<AttestationReport>;

    /// Stop a TEE deployment (may be resumable depending on backend).
    async fn stop(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Destroy a TEE deployment and clean up all resources.
    async fn destroy(&self, deployment_id: &str) -> crate::error::Result<()>;

    /// Which TEE type this backend provides.
    fn tee_type(&self) -> TeeType;

    /// Whether this backend can embed caller-supplied report data in fresh
    /// attestations. Freshness challenges must fail closed when unsupported.
    fn supports_attestation_report_data(&self) -> bool {
        false
    }

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
// Global TEE backend singleton
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) static TEE_BACKEND: once_cell::sync::OnceCell<std::sync::Arc<dyn TeeBackend>> =
    once_cell::sync::OnceCell::new();

/// Initialize the global TEE backend. Call once at startup.
pub fn init_tee_backend(backend: std::sync::Arc<dyn TeeBackend>) {
    if TEE_BACKEND.set(backend).is_err() {
        tracing::warn!("TEE backend already initialized, ignoring duplicate init");
    }
}

/// Get the global TEE backend.
///
/// Returns an error if the backend has not been initialized via
/// [`init_tee_backend`]. Prefer [`try_tee_backend`] when absence is
/// expected (e.g. non-TEE operators).
pub fn tee_backend() -> crate::error::Result<&'static std::sync::Arc<dyn TeeBackend>> {
    TEE_BACKEND.get().ok_or_else(|| {
        crate::error::SandboxError::Validation(
            "TEE backend not initialized — call init_tee_backend() first".into(),
        )
    })
}

/// Try to get the global TEE backend, returning `None` if not initialized.
///
/// Use this in shared code paths (e.g. instance operator API) that need to
/// auto-detect whether TEE is available without panicking.
pub fn try_tee_backend() -> Option<&'static std::sync::Arc<dyn TeeBackend>> {
    TEE_BACKEND.get()
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers for cloud TEE backends
// ─────────────────────────────────────────────────────────────────────────────
