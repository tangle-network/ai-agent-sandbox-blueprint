//! Direct TEE backend skeleton for operators running their own TEE hardware.
//!
//! This backend is for operators who run Docker with SGX device passthrough,
//! TDX/SEV confidential VMs, or Gramine shielded containers on their own
//! infrastructure — as opposed to deploying to a managed service like Phala.
//!
//! # What operators need to implement
//!
//! 1. **Container launch with TEE isolation** — e.g. `docker run --device /dev/sgx_enclave`
//!    or launching inside a TDX/SEV VM with appropriate kernel and firmware support.
//!
//! 2. **Local attestation service** — an endpoint that can produce SGX quotes,
//!    TDX reports, or SEV attestation reports for running containers.
//!
//! 3. **Measurement extraction** — reading MRENCLAVE/MRSIGNER (SGX), MRTD (TDX),
//!    or launch digest (SEV-SNP) from the running enclave.
//!
//! All methods currently return `unimplemented!()` — fill them in based on
//! your hardware and attestation infrastructure.

use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::Result;

/// TEE backend for operators running their own TEE hardware (SGX, TDX, SEV-SNP).
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
        unimplemented!("DirectTeeBackend::deploy — implement for your TEE hardware")
    }

    async fn attestation(&self, _deployment_id: &str) -> Result<AttestationReport> {
        // TODO: Query your local attestation service for a fresh report.
        unimplemented!("DirectTeeBackend::attestation — implement for your TEE hardware")
    }

    async fn stop(&self, _deployment_id: &str) -> Result<()> {
        // TODO: Stop the TEE-isolated container.
        unimplemented!("DirectTeeBackend::stop — implement for your TEE hardware")
    }

    async fn destroy(&self, _deployment_id: &str) -> Result<()> {
        // TODO: Remove the TEE-isolated container and clean up resources.
        unimplemented!("DirectTeeBackend::destroy — implement for your TEE hardware")
    }

    fn tee_type(&self) -> TeeType {
        self.tee_type.clone()
    }
}
