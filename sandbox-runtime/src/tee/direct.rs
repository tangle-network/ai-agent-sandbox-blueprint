//! Direct TEE backend skeleton for operators running their own TEE hardware.
//!
//! This backend is for operators who run TDX/SEV confidential VMs on their own
//! infrastructure — as opposed to deploying to a managed cloud service.
//!
//! # What operators need to implement
//!
//! 1. **Container launch with TEE isolation** — launching inside a TDX/SEV VM
//!    with appropriate kernel and firmware support.
//!
//! 2. **Local attestation service** — an endpoint that can produce TDX reports
//!    or SEV attestation reports for running containers.
//!
//! 3. **Measurement extraction** — reading MRTD (TDX) or launch digest (SEV-SNP)
//!    from the running confidential VM.
//!
//! All methods currently return `unimplemented!()` — fill them in based on
//! your hardware and attestation infrastructure.

use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::Result;

/// TEE backend for operators running their own TEE hardware (TDX, SEV-SNP).
pub struct DirectTeeBackend {
    /// Which TEE technology this operator provides.
    pub tee_type: TeeType,
}

impl DirectTeeBackend {
    pub fn new(tee_type: TeeType) -> Self {
        Self { tee_type }
    }
}

#[async_trait::async_trait]
impl TeeBackend for DirectTeeBackend {
    async fn deploy(&self, _params: &TeeDeployParams) -> Result<TeeDeployment> {
        // TODO: Launch a Docker container with TEE device passthrough,
        // or start a Gramine-shielded container, then extract attestation.
        Err(crate::error::SandboxError::Validation(
            "DirectTeeBackend::deploy not implemented — configure your TEE hardware".into(),
        ))
    }

    async fn attestation(&self, _deployment_id: &str) -> Result<AttestationReport> {
        // TODO: Query your local attestation service for a fresh report.
        Err(crate::error::SandboxError::Validation(
            "DirectTeeBackend::attestation not implemented — configure your TEE hardware".into(),
        ))
    }

    async fn stop(&self, _deployment_id: &str) -> Result<()> {
        // TODO: Stop the TEE-isolated container.
        Err(crate::error::SandboxError::Validation(
            "DirectTeeBackend::stop not implemented — configure your TEE hardware".into(),
        ))
    }

    async fn destroy(&self, _deployment_id: &str) -> Result<()> {
        // TODO: Remove the TEE-isolated container and clean up resources.
        Err(crate::error::SandboxError::Validation(
            "DirectTeeBackend::destroy not implemented — configure your TEE hardware".into(),
        ))
    }

    fn tee_type(&self) -> TeeType {
        self.tee_type.clone()
    }
}
