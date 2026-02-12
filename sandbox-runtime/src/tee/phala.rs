//! Phala dstack TEE backend.
//!
//! Deploys sidecar containers as Phala CVMs via `phala-tee-deploy-rs`.
//! The sidecar image is wrapped in a docker-compose.yml and deployed
//! to Phala Cloud, which runs it inside a TDX-based confidential VM.

use std::collections::HashMap;
use std::time::Duration;

use phala_tee_deploy_rs::{TeeDeployer, TeeDeployerBuilder};

use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};

/// TEE backend that deploys containers to Phala Cloud CVMs.
pub struct PhalaBackend {
    deployer: TeeDeployer,
}

impl PhalaBackend {
    /// Create a new Phala backend using the given API key.
    ///
    /// Optionally provide a custom API endpoint (defaults to Phala Cloud production).
    pub fn new(api_key: &str, api_endpoint: Option<String>) -> Result<Self> {
        let mut builder = TeeDeployerBuilder::new().with_api_key(api_key);
        if let Some(endpoint) = api_endpoint {
            builder = builder.with_api_endpoint(endpoint);
        }
        let deployer = builder
            .build()
            .map_err(|e| SandboxError::Validation(format!("Failed to create Phala deployer: {e}")))?;
        Ok(Self { deployer })
    }

    /// Build a docker-compose YAML wrapping the sidecar image.
    fn compose_yaml(params: &TeeDeployParams) -> String {
        let mut yaml = String::from("services:\n  sidecar:\n");
        yaml.push_str(&format!("    image: {}\n", params.image));

        // Ports
        yaml.push_str("    ports:\n");
        yaml.push_str(&format!(
            "      - \"{}:{}\"\n",
            params.http_port, params.http_port
        ));
        if let Some(ssh) = params.ssh_port {
            yaml.push_str(&format!("      - \"{}:22\"\n", ssh));
        }

        // Environment
        if !params.env_vars.is_empty() {
            yaml.push_str("    environment:\n");
            for (k, v) in &params.env_vars {
                yaml.push_str(&format!("      - {}={}\n", k, v));
            }
        }

        // Mount dstack socket for attestation inside the CVM
        yaml.push_str("    volumes:\n");
        yaml.push_str("      - /var/run/dstack.sock:/var/run/dstack.sock\n");

        yaml
    }
}

#[async_trait::async_trait]
impl TeeBackend for PhalaBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let compose = Self::compose_yaml(params);

        let env_vars: HashMap<String, String> = params
            .env_vars
            .iter()
            .cloned()
            .collect();

        let app_name = format!("sandbox-{}", &params.sandbox_id);

        let deployment = self
            .deployer
            .deploy_compose(
                &compose,
                &app_name,
                env_vars,
                Some(params.cpu_cores.max(1)),
                Some(params.memory_mb.max(1024)),
                Some(params.disk_gb.max(10)),
            )
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala deploy failed: {e}")))?;

        let app_id = deployment.id.to_string();

        // Wait for the CVM to become running.
        self.deployer
            .wait_until_running(&app_id, Duration::from_secs(120))
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala CVM failed to start: {e}")))?;

        // Fetch attestation.
        let att_resp = self
            .deployer
            .get_attestation(&app_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala attestation fetch failed: {e}")))?;

        let attestation = AttestationReport {
            tee_type: TeeType::Sgx,
            evidence: serde_json::to_vec(&att_resp.tcb_info).unwrap_or_default(),
            measurement: serde_json::to_vec(&att_resp.app_certificates).unwrap_or_default(),
            timestamp: crate::util::now_ts(),
        };

        // Get network info for the sidecar URL.
        let network = self
            .deployer
            .get_network_info(&app_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala network info failed: {e}")))?;

        let sidecar_url = if !network.public_urls.app.is_empty() {
            network.public_urls.app.clone()
        } else {
            format!("http://{}:{}", network.internal_ip, params.http_port)
        };

        let metadata = serde_json::json!({
            "phala_app_id": app_id,
            "phala_internal_ip": network.internal_ip,
            "phala_public_url": network.public_urls.app,
        });

        Ok(TeeDeployment {
            deployment_id: app_id,
            sidecar_url,
            ssh_port: params.ssh_port,
            attestation,
            metadata_json: metadata.to_string(),
        })
    }

    async fn attestation(&self, deployment_id: &str) -> Result<AttestationReport> {
        let att_resp = self
            .deployer
            .get_attestation(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala attestation fetch failed: {e}")))?;

        Ok(AttestationReport {
            tee_type: TeeType::Sgx,
            evidence: serde_json::to_vec(&att_resp.tcb_info).unwrap_or_default(),
            measurement: serde_json::to_vec(&att_resp.app_certificates).unwrap_or_default(),
            timestamp: crate::util::now_ts(),
        })
    }

    async fn stop(&self, deployment_id: &str) -> Result<()> {
        self.deployer
            .shutdown(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala shutdown failed: {e}")))?;
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        // Graceful shutdown first, then delete.
        let _ = self.deployer.shutdown(deployment_id).await;
        self.deployer
            .delete(deployment_id)
            .await
            .map_err(|e| SandboxError::Docker(format!("Phala delete failed: {e}")))?;
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        TeeType::Sgx
    }
}
