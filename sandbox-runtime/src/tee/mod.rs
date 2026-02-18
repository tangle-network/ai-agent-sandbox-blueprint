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

// ─────────────────────────────────────────────────────────────────────────────
// Mock backend for tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "test-utils"))]
pub mod mock {
    use super::*;
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
        async fn deploy(
            &self,
            params: &TeeDeployParams,
        ) -> crate::error::Result<TeeDeployment> {
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
            })
        }

        async fn attestation(
            &self,
            _deployment_id: &str,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn tee_type_serialization_roundtrip() {
        for variant in [TeeType::None, TeeType::Tdx, TeeType::Nitro, TeeType::Sev] {
            let json = serde_json::to_string(&variant).unwrap();
            let decoded: TeeType = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, variant);
        }
    }

    #[test]
    fn attestation_report_serialization() {
        let report = AttestationReport {
            tee_type: TeeType::Tdx,
            evidence: vec![0xDE, 0xAD, 0xBE, 0xEF],
            measurement: vec![0x01, 0x02, 0x03],
            timestamp: 1_700_000_000,
        };
        let json = serde_json::to_string(&report).unwrap();
        let decoded: AttestationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.tee_type, TeeType::Tdx);
        assert_eq!(decoded.evidence, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(decoded.measurement, vec![0x01, 0x02, 0x03]);
        assert_eq!(decoded.timestamp, 1_700_000_000);
    }

    #[test]
    fn tee_deploy_params_from_sandbox_params() {
        let params = crate::runtime::CreateSandboxParams {
            name: "test".into(),
            image: "my-image:latest".into(),
            env_json: r#"{"API_KEY":"secret","COUNT":42,"VERBOSE":true}"#.into(),
            ssh_enabled: true,
            cpu_cores: 4,
            memory_mb: 8192,
            disk_gb: 100,
            ..Default::default()
        };

        let deploy = TeeDeployParams::from_sandbox_params("sb-1", &params, 8080, 2222, "tok-abc");

        assert_eq!(deploy.sandbox_id, "sb-1");
        assert_eq!(deploy.image, "my-image:latest");
        assert_eq!(deploy.http_port, 8080);
        assert_eq!(deploy.ssh_port, Some(2222));
        assert_eq!(deploy.sidecar_token, "tok-abc");
        assert_eq!(deploy.cpu_cores, 4);
        assert_eq!(deploy.memory_mb, 8192);
        assert_eq!(deploy.disk_gb, 100);

        // Check env vars: SIDECAR_PORT + SIDECAR_AUTH_TOKEN + 3 from env_json
        assert_eq!(deploy.env_vars.len(), 5);
        assert!(deploy.env_vars.contains(&("SIDECAR_PORT".into(), "8080".into())));
        assert!(deploy.env_vars.contains(&("SIDECAR_AUTH_TOKEN".into(), "tok-abc".into())));
        assert!(deploy.env_vars.contains(&("API_KEY".into(), "secret".into())));
        assert!(deploy.env_vars.contains(&("COUNT".into(), "42".into())));
        assert!(deploy.env_vars.contains(&("VERBOSE".into(), "true".into())));
    }

    #[test]
    fn tee_deploy_params_ssh_disabled() {
        let params = crate::runtime::CreateSandboxParams {
            ssh_enabled: false,
            ..Default::default()
        };
        let deploy = TeeDeployParams::from_sandbox_params("sb-2", &params, 8080, 2222, "tok");
        assert_eq!(deploy.ssh_port, None);
    }

    #[test]
    fn tee_deploy_params_skips_nested_objects() {
        let params = crate::runtime::CreateSandboxParams {
            env_json: r#"{"SIMPLE":"val","NESTED":{"a":1},"ARR":[1,2]}"#.into(),
            ..Default::default()
        };
        let deploy = TeeDeployParams::from_sandbox_params("sb-3", &params, 8080, 22, "t");
        // Only SIDECAR_PORT + SIDECAR_AUTH_TOKEN + SIMPLE (nested/array skipped)
        assert_eq!(deploy.env_vars.len(), 3);
        assert!(deploy.env_vars.contains(&("SIMPLE".into(), "val".into())));
    }

    #[tokio::test]
    async fn mock_backend_deploy_and_lifecycle() {
        let mock = mock::MockTeeBackend::new(TeeType::Tdx);

        let params = TeeDeployParams {
            sandbox_id: "sb-test".into(),
            image: "test:latest".into(),
            env_vars: vec![],
            cpu_cores: 2,
            memory_mb: 4096,
            disk_gb: 50,
            http_port: 8080,
            ssh_port: Some(2222),
            sidecar_token: "tok".into(),
        };

        // Deploy
        let deployment = mock.deploy(&params).await.unwrap();
        assert_eq!(deployment.deployment_id, "mock-deploy-sb-test");
        assert_eq!(deployment.sidecar_url, "http://mock-tee:8080");
        assert_eq!(deployment.ssh_port, Some(2222));
        assert_eq!(deployment.attestation.tee_type, TeeType::Tdx);
        assert_eq!(mock.deploy_count.load(Ordering::Relaxed), 1);

        // Attestation
        let att = mock.attestation("mock-deploy-sb-test").await.unwrap();
        assert_eq!(att.tee_type, TeeType::Tdx);
        assert_eq!(mock.attestation_count.load(Ordering::Relaxed), 1);

        // Stop
        mock.stop("mock-deploy-sb-test").await.unwrap();
        assert_eq!(mock.stop_count.load(Ordering::Relaxed), 1);

        // Destroy
        mock.destroy("mock-deploy-sb-test").await.unwrap();
        assert_eq!(mock.destroy_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn mock_backend_failing_mode() {
        let mock = mock::MockTeeBackend::failing(TeeType::Nitro);

        let params = TeeDeployParams {
            sandbox_id: "sb-fail".into(),
            image: "test:latest".into(),
            env_vars: vec![],
            cpu_cores: 1,
            memory_mb: 1024,
            disk_gb: 10,
            http_port: 8080,
            ssh_port: None,
            sidecar_token: "tok".into(),
        };

        assert!(mock.deploy(&params).await.is_err());
        assert!(mock.attestation("x").await.is_err());
        assert!(mock.stop("x").await.is_err());
        assert!(mock.destroy("x").await.is_err());
    }

    #[tokio::test]
    async fn mock_backend_sealed_secrets_supported() {
        let mock = mock::MockTeeBackend::new(TeeType::Tdx);

        let pk = mock.derive_public_key("dep-1").await.unwrap();
        assert_eq!(pk.algorithm, "x25519-hkdf-sha256");
        assert_eq!(mock.derive_pk_count.load(Ordering::Relaxed), 1);

        let sealed = sealed_secrets::SealedSecret {
            algorithm: "x25519-xsalsa20-poly1305".into(),
            ciphertext: vec![0xAA],
            nonce: vec![0xBB],
        };
        let result = mock.inject_sealed_secrets("dep-1", &sealed).await.unwrap();
        assert!(result.success);
        assert_eq!(result.secrets_count, 3);
        assert_eq!(mock.inject_secrets_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn mock_backend_sealed_secrets_unsupported() {
        let mock = mock::MockTeeBackend::new(TeeType::Tdx);
        mock.support_sealed_secrets
            .store(false, Ordering::Relaxed);

        assert!(mock.derive_public_key("dep-1").await.is_err());
        assert!(mock
            .inject_sealed_secrets(
                "dep-1",
                &sealed_secrets::SealedSecret {
                    algorithm: "test".into(),
                    ciphertext: vec![],
                    nonce: vec![],
                }
            )
            .await
            .is_err());
    }
}
