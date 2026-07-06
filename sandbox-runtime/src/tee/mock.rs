//! Mock TEE backend for tests / test-utils.

use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A configurable mock TEE backend for tests.
///
/// Tracks call counts via atomics. By default all operations succeed and
/// sealed secrets are supported. Use `failing()` to create a mock that
/// returns errors for all operations.
pub struct MockTeeBackend {
    pub tee_type: TeeType,
    pub deploy_count: AtomicUsize,
    pub stop_count: AtomicUsize,
    pub destroy_count: AtomicUsize,
    pub attestation_count: AtomicUsize,
    pub derive_pk_count: AtomicUsize,
    pub inject_secrets_count: AtomicUsize,
    pub should_fail: AtomicBool,
    pub support_sealed_secrets: AtomicBool,
    pub support_report_data: AtomicBool,
}

impl MockTeeBackend {
    pub fn new(tee_type: TeeType) -> Self {
        Self {
            tee_type,
            deploy_count: AtomicUsize::new(0),
            stop_count: AtomicUsize::new(0),
            destroy_count: AtomicUsize::new(0),
            attestation_count: AtomicUsize::new(0),
            derive_pk_count: AtomicUsize::new(0),
            inject_secrets_count: AtomicUsize::new(0),
            should_fail: AtomicBool::new(false),
            support_sealed_secrets: AtomicBool::new(true),
            support_report_data: AtomicBool::new(true),
        }
    }

    pub fn failing(tee_type: TeeType) -> Self {
        let mock = Self::new(tee_type);
        mock.should_fail.store(true, Ordering::Relaxed);
        mock
    }

    fn dummy_attestation(&self) -> AttestationReport {
        AttestationReport {
            tee_type: self.tee_type.clone(),
            evidence: vec![0xDE, 0xAD],
            measurement: vec![0xBE, 0xEF],
            timestamp: 1_700_000_000,
        }
    }
}

#[async_trait::async_trait]
impl TeeBackend for MockTeeBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> crate::error::Result<TeeDeployment> {
        self.deploy_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::CloudProvider(
                "Mock deploy failure".into(),
            ));
        }
        Ok(TeeDeployment {
            deployment_id: format!("mock-deploy-{}", params.sandbox_id),
            sidecar_url: format!("http://mock-tee:{}", params.http_port),
            ssh_port: params.ssh_port,
            attestation: self.dummy_attestation(),
            metadata_json: r#"{"backend":"mock"}"#.to_string(),
            extra_ports: HashMap::new(),
        })
    }

    async fn attestation(
        &self,
        _deployment_id: &str,
        _report_data: Option<[u8; 64]>,
    ) -> crate::error::Result<AttestationReport> {
        self.attestation_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::CloudProvider(
                "Mock attestation failure".into(),
            ));
        }
        Ok(self.dummy_attestation())
    }

    async fn stop(&self, _deployment_id: &str) -> crate::error::Result<()> {
        self.stop_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::CloudProvider(
                "Mock stop failure".into(),
            ));
        }
        Ok(())
    }

    async fn destroy(&self, _deployment_id: &str) -> crate::error::Result<()> {
        self.destroy_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::CloudProvider(
                "Mock destroy failure".into(),
            ));
        }
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        self.tee_type.clone()
    }

    fn supports_attestation_report_data(&self) -> bool {
        self.support_report_data.load(Ordering::Relaxed)
    }

    async fn derive_public_key(
        &self,
        _deployment_id: &str,
    ) -> crate::error::Result<sealed_secrets::TeePublicKey> {
        self.derive_pk_count.fetch_add(1, Ordering::Relaxed);
        if !self.support_sealed_secrets.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::Validation(
                "Sealed secrets not supported by mock".into(),
            ));
        }
        Ok(sealed_secrets::TeePublicKey {
            algorithm: "x25519-hkdf-sha256".to_string(),
            public_key_bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
            attestation: self.dummy_attestation(),
        })
    }

    async fn inject_sealed_secrets(
        &self,
        _deployment_id: &str,
        _sealed: &sealed_secrets::SealedSecret,
    ) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
        self.inject_secrets_count.fetch_add(1, Ordering::Relaxed);
        if !self.support_sealed_secrets.load(Ordering::Relaxed) {
            return Err(crate::error::SandboxError::Validation(
                "Sealed secrets not supported by mock".into(),
            ));
        }
        Ok(sealed_secrets::SealedSecretResult {
            success: true,
            secrets_count: 3,
            error: None,
        })
    }
}
