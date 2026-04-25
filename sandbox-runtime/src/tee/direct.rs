//! Direct TEE backend for operators running their own TEE hardware.
//!
//! Deploys sidecar containers via the local Docker daemon with TEE device
//! passthrough. The operator's host must be a confidential VM (TDX, SEV-SNP)
//! or have TEE device nodes available (`/dev/tdx_guest`, `/dev/sev-guest`,
//! `/dev/nsm`).
//!
//! Attestation is fetched from the sidecar's `/tee/attestation` endpoint,
//! which queries the host TEE device from inside the container.
//!
//! # Required environment
//!
//! - `TEE_DIRECT_TYPE` — `tdx`, `sev`, or `nitro`
//! - Standard sidecar env vars (`SIDECAR_IMAGE`, `SIDECAR_PUBLIC_HOST`, etc.)
//!
//! # Device passthrough
//!
//! | TEE type | Device node         |
//! |----------|---------------------|
//! | TDX      | `/dev/tdx_guest`    |
//! | SEV-SNP  | `/dev/sev-guest`    |
//! | Nitro    | `/dev/nsm`          |

use std::collections::HashMap;
use std::time::Duration;

use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions, StopContainerOptions,
};
use docktopus::bollard::models::{DeviceMapping, HostConfig, PortBinding, PortMap};
use docktopus::container::Container;

use super::sealed_secrets::{SealedSecret, SealedSecretResult, TeePublicKey};
use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};
use crate::runtime::{SidecarRuntimeConfig, docker_builder, docker_timeout};

/// Metadata stored in `TeeDeployment.metadata_json` for later lifecycle ops.
#[derive(serde::Serialize, serde::Deserialize)]
struct DirectMetadata {
    container_id: String,
    device_path: String,
}

/// TEE backend for operators running their own TEE hardware (TDX, SEV-SNP, Nitro).
///
/// Launches Docker containers with the appropriate TEE device node passed through,
/// allowing the sidecar to produce hardware attestation reports from inside the
/// container.
pub struct DirectTeeBackend {
    /// Which TEE technology this operator provides.
    pub tee_type: TeeType,
    /// Skip TEE device passthrough (for testing on non-TEE hosts).
    skip_device: bool,
}

impl DirectTeeBackend {
    pub fn new(tee_type: TeeType) -> Self {
        Self {
            tee_type,
            skip_device: false,
        }
    }

    /// Create a backend that skips TEE device passthrough, allowing containers
    /// to start on hosts without TEE hardware. For integration testing only.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn new_without_device(tee_type: TeeType) -> Self {
        Self {
            tee_type,
            skip_device: true,
        }
    }

    /// Returns the host device path for the configured TEE type.
    fn device_path(&self) -> &'static str {
        match self.tee_type {
            TeeType::Tdx => "/dev/tdx_guest",
            TeeType::Sev => "/dev/sev-guest",
            TeeType::Nitro => "/dev/nsm",
            TeeType::None => {
                unreachable!("DirectTeeBackend should not be created with TeeType::None")
            }
        }
    }

    /// Build a Docker container config with TEE device passthrough.
    fn build_config(&self, params: &TeeDeployParams) -> BollardConfig<String> {
        let config = SidecarRuntimeConfig::load();

        // Port bindings — bind to localhost only, let Docker assign host ports.
        let mut port_bindings = PortMap::new();
        port_bindings.insert(
            format!("{}/tcp", params.http_port),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: None,
            }]),
        );
        if let Some(ssh) = params.ssh_port {
            port_bindings.insert(
                format!("{ssh}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".to_string()),
                    host_port: None,
                }]),
            );
        }
        for &port in &params.extra_ports {
            port_bindings.insert(
                format!("{port}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".to_string()),
                    host_port: None,
                }]),
            );
        }

        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        exposed_ports.insert(format!("{}/tcp", params.http_port), HashMap::new());
        if let Some(ssh) = params.ssh_port {
            exposed_ports.insert(format!("{ssh}/tcp"), HashMap::new());
        }
        for &port in &params.extra_ports {
            exposed_ports.insert(format!("{port}/tcp"), HashMap::new());
        }

        // TEE device passthrough (skipped in test mode).
        let devices = if self.skip_device {
            vec![]
        } else {
            let device_path = self.device_path().to_string();
            vec![DeviceMapping {
                path_on_host: Some(device_path.clone()),
                path_in_container: Some(device_path),
                cgroup_permissions: Some("rwm".to_string()),
            }]
        };

        let mut host_config = HostConfig {
            port_bindings: Some(port_bindings),
            devices: if devices.is_empty() {
                None
            } else {
                Some(devices)
            },
            cap_drop: Some(vec!["ALL".to_string()]),
            cap_add: Some(vec!["SYS_PTRACE".to_string()]),
            security_opt: Some(vec!["no-new-privileges=true".to_string()]),
            pids_limit: Some(512),
            readonly_rootfs: Some(true),
            tmpfs: Some(HashMap::from([
                ("/tmp".to_string(), "rw,noexec,nosuid,size=512m".to_string()),
                ("/run".to_string(), "rw,noexec,nosuid,size=64m".to_string()),
            ])),
            ..Default::default()
        };

        if params.cpu_cores > 0 {
            host_config.nano_cpus = Some((params.cpu_cores as i64) * 1_000_000_000);
        }
        if params.memory_mb > 0 {
            host_config.memory = Some((params.memory_mb as i64) * 1024 * 1024);
        }

        let _ = config; // accessed only for consistency; params carry port info

        BollardConfig {
            exposed_ports: Some(exposed_ports),
            host_config: Some(host_config),
            ..Default::default()
        }
    }

    /// Extract host ports from a running container's inspect response.
    fn extract_host_port(
        ports: &HashMap<String, Option<Vec<PortBinding>>>,
        container_port: u16,
    ) -> Result<u16> {
        let key = format!("{container_port}/tcp");
        let bindings = ports
            .get(&key)
            .and_then(|v| v.as_ref())
            .ok_or_else(|| SandboxError::Docker(format!("Missing port bindings for {key}")))?;

        let binding = bindings
            .first()
            .ok_or_else(|| SandboxError::Docker(format!("Empty port bindings for {key}")))?;

        binding
            .host_port
            .as_deref()
            .and_then(|p| p.split('/').next()) // strip /tcp suffix if present
            .and_then(|p| p.parse::<u16>().ok())
            .ok_or_else(|| SandboxError::Docker(format!("Invalid host port for {key}")))
    }
}

#[async_trait::async_trait]
impl TeeBackend for DirectTeeBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        if params.attestation_report_data.is_some() && self.tee_type == TeeType::Tdx {
            return Err(SandboxError::Validation(
                "Direct TDX nonce-bound remote attestation requires a DCAP TD quote; /dev/tdx_guest TDX_CMD_GET_REPORT0 returns a local TDREPORT only".into(),
            ));
        }

        let builder = docker_builder().await?;
        let config = SidecarRuntimeConfig::load();

        // Pull image if configured.
        if config.pull_image {
            let _ = docker_timeout("pull_image", builder.pull_image(&params.image, None)).await;
        }

        let container_name = format!("tee-direct-{}", params.sandbox_id);
        let docker_config = self.build_config(params);
        let env_vars: Vec<String> = params
            .env_vars
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let mut container = Container::new(builder.client(), params.image.clone())
            .with_name(container_name)
            .env(env_vars)
            .config_override(docker_config);

        // Start with retry for transient Docker failures.
        match docker_timeout("start_container", container.start(false)).await {
            Ok(()) => {}
            Err(first_err) => {
                tracing::warn!(error = %first_err, "Direct TEE container start failed, retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
                docker_timeout("start_container_retry", container.start(false)).await?;
            }
        }

        let container_id = container
            .id()
            .ok_or_else(|| SandboxError::Docker("Missing container id after start".into()))?
            .to_string();

        // Inspect to get port mappings.
        let inspect = docker_timeout(
            "inspect_container",
            builder
                .client()
                .inspect_container(&container_id, None::<InspectContainerOptions>),
        )
        .await?;

        let ports = inspect
            .network_settings
            .as_ref()
            .and_then(|s| s.ports.as_ref())
            .ok_or_else(|| SandboxError::Docker("Missing port mappings".into()))?;

        let host_port = Self::extract_host_port(ports, params.http_port)?;
        let ssh_host_port = params
            .ssh_port
            .map(|p| Self::extract_host_port(ports, p))
            .transpose()?;

        let mut extra_port_map = HashMap::new();
        for &cp in &params.extra_ports {
            if let Ok(hp) = Self::extract_host_port(ports, cp) {
                extra_port_map.insert(cp, hp);
            }
        }

        let sidecar_url = format!("http://{}:{host_port}", config.public_host);

        // Wait for sidecar to become healthy.
        super::wait_for_sidecar_health(
            &sidecar_url,
            &params.sidecar_token,
            Duration::from_secs(60),
        )
        .await?;

        // Try native attestation first, fall back to sidecar.
        let nonce = params.attestation_report_data.unwrap_or_else(|| {
            let mut nonce = [0u8; 64];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
            nonce
        });
        let attestation = match super::attestation::generate_native_attestation(
            &self.tee_type,
            &nonce,
        ) {
            Ok(att) => {
                tracing::info!("Native TEE attestation generated successfully");
                att
            }
            Err(native_err) => {
                if params.attestation_report_data.is_some() {
                    return Err(native_err);
                }
                tracing::warn!(error = %native_err, "Native attestation unavailable, falling back to sidecar");
                super::fetch_sidecar_attestation(&sidecar_url, &params.sidecar_token).await?
            }
        };

        let metadata = DirectMetadata {
            container_id: container_id.clone(),
            device_path: self.device_path().to_string(),
        };

        Ok(TeeDeployment {
            deployment_id: container_id,
            sidecar_url,
            ssh_port: ssh_host_port,
            attestation,
            metadata_json: serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string()),
            extra_ports: extra_port_map,
        })
    }

    async fn attestation(
        &self,
        deployment_id: &str,
        report_data: Option<[u8; 64]>,
    ) -> Result<AttestationReport> {
        if report_data.is_some() && self.tee_type == TeeType::Tdx {
            return Err(SandboxError::Validation(
                "Direct TDX nonce-bound remote attestation requires a DCAP TD quote; /dev/tdx_guest TDX_CMD_GET_REPORT0 returns a local TDREPORT only".into(),
            ));
        }

        let nonce = report_data.unwrap_or_else(|| {
            let mut nonce = [0u8; 64];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
            nonce
        });
        match super::attestation::generate_native_attestation(&self.tee_type, &nonce) {
            Ok(att) => Ok(att),
            Err(err) => {
                if report_data.is_some() {
                    return Err(err);
                }
                let (sidecar_url, token) = super::sidecar_info_for_deployment(deployment_id)?;
                super::fetch_sidecar_attestation(&sidecar_url, &token).await
            }
        }
    }

    async fn stop(&self, deployment_id: &str) -> Result<()> {
        let builder = docker_builder().await?;
        docker_timeout(
            "stop_container",
            builder
                .client()
                .stop_container(deployment_id, Some(StopContainerOptions { t: 30 })),
        )
        .await?;
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        let builder = docker_builder().await?;

        // Graceful stop first, ignore errors (may already be stopped).
        let _ = docker_timeout(
            "stop_container",
            builder
                .client()
                .stop_container(deployment_id, Some(StopContainerOptions { t: 10 })),
        )
        .await;

        docker_timeout(
            "remove_container",
            builder.client().remove_container(
                deployment_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            ),
        )
        .await?;
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        self.tee_type.clone()
    }

    fn supports_attestation_report_data(&self) -> bool {
        matches!(self.tee_type, TeeType::Sev)
    }

    // ── Sealed secrets ──────────────────────────────────────────────────────

    async fn derive_public_key(&self, deployment_id: &str) -> Result<TeePublicKey> {
        super::sidecar_derive_public_key(deployment_id).await
    }

    async fn inject_sealed_secrets(
        &self,
        deployment_id: &str,
        sealed: &SealedSecret,
    ) -> Result<SealedSecretResult> {
        super::sidecar_inject_sealed_secrets(deployment_id, sealed).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_path_tdx() {
        let backend = DirectTeeBackend::new(TeeType::Tdx);
        assert_eq!(backend.device_path(), "/dev/tdx_guest");
    }

    #[test]
    fn device_path_sev() {
        let backend = DirectTeeBackend::new(TeeType::Sev);
        assert_eq!(backend.device_path(), "/dev/sev-guest");
    }

    #[test]
    fn device_path_nitro() {
        let backend = DirectTeeBackend::new(TeeType::Nitro);
        assert_eq!(backend.device_path(), "/dev/nsm");
    }

    #[test]
    fn tee_type_roundtrip() {
        for tt in [TeeType::Tdx, TeeType::Sev, TeeType::Nitro] {
            let backend = DirectTeeBackend::new(tt.clone());
            assert_eq!(backend.tee_type(), tt);
        }
    }

    #[test]
    fn report_data_support_is_limited_to_remotely_verifiable_direct_backends() {
        assert!(!DirectTeeBackend::new(TeeType::Tdx).supports_attestation_report_data());
        assert!(DirectTeeBackend::new(TeeType::Sev).supports_attestation_report_data());
        assert!(!DirectTeeBackend::new(TeeType::Nitro).supports_attestation_report_data());
    }

    #[tokio::test]
    async fn direct_tdx_rejects_nonce_bound_attestation_without_dcap_quote() {
        let backend = DirectTeeBackend::new(TeeType::Tdx);
        let result = backend.attestation("missing", Some([7u8; 64])).await;

        assert!(matches!(
            result,
            Err(SandboxError::Validation(message))
                if message.contains("DCAP TD quote")
                    && message.contains("TDREPORT")
        ));
    }

    #[test]
    fn metadata_serialization() {
        let meta = DirectMetadata {
            container_id: "abc123".into(),
            device_path: "/dev/tdx_guest".into(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let decoded: DirectMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.container_id, "abc123");
        assert_eq!(decoded.device_path, "/dev/tdx_guest");
    }

    #[test]
    fn build_config_includes_device() {
        let backend = DirectTeeBackend::new(TeeType::Tdx);
        let params = TeeDeployParams {
            sandbox_id: "test-sb".into(),
            image: "test:latest".into(),
            env_vars: vec![],
            cpu_cores: 2,
            memory_mb: 4096,
            disk_gb: 50,
            http_port: 3000,
            ssh_port: Some(2222),
            sidecar_token: "tok".into(),
            extra_ports: vec![],
            attestation_report_data: None,
        };

        let config = backend.build_config(&params);

        // Verify device passthrough is present.
        let host_config = config.host_config.unwrap();
        let devices = host_config.devices.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/tdx_guest"));
        assert_eq!(
            devices[0].path_in_container.as_deref(),
            Some("/dev/tdx_guest")
        );
        assert_eq!(devices[0].cgroup_permissions.as_deref(), Some("rwm"));

        // Verify security hardening is preserved.
        assert_eq!(host_config.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(host_config.cap_add, Some(vec!["SYS_PTRACE".to_string()]));
        assert_eq!(host_config.pids_limit, Some(512));
        assert_eq!(host_config.readonly_rootfs, Some(true));

        // Verify resource constraints.
        assert_eq!(host_config.nano_cpus, Some(2_000_000_000));
        assert_eq!(host_config.memory, Some(4096 * 1024 * 1024));

        // Verify port bindings.
        let port_bindings = host_config.port_bindings.unwrap();
        assert!(port_bindings.contains_key("3000/tcp"));
        assert!(port_bindings.contains_key("2222/tcp"));

        // Verify exposed ports.
        let exposed = config.exposed_ports.unwrap();
        assert!(exposed.contains_key("3000/tcp"));
        assert!(exposed.contains_key("2222/tcp"));
    }

    #[test]
    fn build_config_no_ssh() {
        let backend = DirectTeeBackend::new(TeeType::Sev);
        let params = TeeDeployParams {
            sandbox_id: "test-sb".into(),
            image: "test:latest".into(),
            env_vars: vec![],
            cpu_cores: 0,
            memory_mb: 0,
            disk_gb: 0,
            http_port: 8080,
            ssh_port: None,
            sidecar_token: "tok".into(),
            extra_ports: vec![],
            attestation_report_data: None,
        };

        let config = backend.build_config(&params);
        let host_config = config.host_config.unwrap();

        // SEV device.
        let devices = host_config.devices.unwrap();
        assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/sev-guest"));

        // No SSH port.
        let port_bindings = host_config.port_bindings.unwrap();
        assert!(port_bindings.contains_key("8080/tcp"));
        assert!(!port_bindings.contains_key("2222/tcp"));

        // Zero resources means no constraints set.
        assert_eq!(host_config.nano_cpus, None);
        assert_eq!(host_config.memory, None);
    }

    #[test]
    fn extract_host_port_success() {
        let mut ports = HashMap::new();
        ports.insert(
            "3000/tcp".to_string(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".into()),
                host_port: Some("49152".into()),
            }]),
        );

        let port = DirectTeeBackend::extract_host_port(&ports, 3000).unwrap();
        assert_eq!(port, 49152);
    }

    #[test]
    fn extract_host_port_missing() {
        let ports = HashMap::new();
        let result = DirectTeeBackend::extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }
}
