//! GCP Confidential Space TEE backend.
//!
//! Deploys sidecar containers on GCP Confidential VMs running the
//! Confidential Space launcher. Supports both AMD SEV-SNP (N2D machines)
//! and Intel TDX (C3 machines).
//!
//! # Deploy flow
//!
//! 1. Create a Compute Engine VM with `confidentialInstanceConfig` and
//!    `tee-image-reference` metadata pointing to the sidecar container image.
//! 2. The Confidential Space launcher auto-pulls the image, starts it inside
//!    the TEE, and provides OIDC attestation tokens via a Unix socket at
//!    `/run/container_launcher/teeserver.sock`.
//! 3. The sidecar exchanges its OIDC attestation token → STS → GCP access
//!    token → Cloud KMS decrypt.
//!
//! # Sealed secrets
//!
//! The sidecar obtains an attestation token from the launcher socket. A WIP
//! (Workload Identity Pool) attribute condition binds Cloud KMS decrypt
//! permissions to `image_digest`, ensuring only the expected container image
//! running in a genuine TEE can access secrets. No key pair needed — Cloud KMS
//! handles encryption/decryption directly.
//!
//! # Machine types
//!
//! | Series | Technology | `confidentialInstanceType` |
//! |--------|-----------|---------------------------|
//! | N2D    | AMD SEV-SNP | `SEV` or `SEV_SNP`     |
//! | C3     | Intel TDX   | `TDX`                  |
//! | C2D    | AMD SEV     | `SEV`                  |

use std::time::Duration;

use tokio::sync::OnceCell;

use super::sealed_secrets::{SealedSecret, SealedSecretResult, TeePublicKey};
use super::{AttestationReport, TeeBackend, TeeDeployParams, TeeDeployment, TeeType};
use crate::error::{Result, SandboxError};

const COMPUTE_BASE: &str = "https://compute.googleapis.com/compute/v1";

/// Configuration for the GCP Confidential Space backend, read from environment variables.
#[derive(Clone, Debug)]
pub struct GcpConfig {
    pub project_id: String,
    pub zone: String,
    pub confidential_space_image: String,
    pub machine_type: String,
    pub service_account_email: Option<String>,
    pub network: Option<String>,
    pub subnet: Option<String>,
    pub kms_key_resource: Option<String>,
}

impl GcpConfig {
    /// Load configuration from environment variables.
    ///
    /// Required: `GCP_PROJECT_ID`, `GCP_ZONE`, `GCP_CONFIDENTIAL_SPACE_IMAGE`.
    /// Optional: `GCP_MACHINE_TYPE` (default: n2d-standard-4),
    /// `GCP_SERVICE_ACCOUNT_EMAIL`, `GCP_NETWORK`, `GCP_SUBNET`,
    /// `GCP_KMS_KEY_RESOURCE`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            project_id: require_env("GCP_PROJECT_ID")?,
            zone: require_env("GCP_ZONE")?,
            confidential_space_image: require_env("GCP_CONFIDENTIAL_SPACE_IMAGE")?,
            machine_type: std::env::var("GCP_MACHINE_TYPE")
                .unwrap_or_else(|_| "n2d-standard-4".to_string()),
            service_account_email: std::env::var("GCP_SERVICE_ACCOUNT_EMAIL").ok(),
            network: std::env::var("GCP_NETWORK").ok(),
            subnet: std::env::var("GCP_SUBNET").ok(),
            kms_key_resource: std::env::var("GCP_KMS_KEY_RESOURCE").ok(),
        })
    }

    /// Infer the TEE type from the GCP machine type prefix.
    /// C3 machines use Intel TDX, all others default to AMD SEV-SNP.
    fn inferred_tee_type(&self) -> TeeType {
        if self.machine_type.starts_with("c3-") {
            TeeType::Tdx
        } else {
            TeeType::Sev
        }
    }

    /// Infer the `confidentialInstanceType` for the Compute Engine API.
    fn confidential_instance_type(&self) -> &'static str {
        if self.machine_type.starts_with("c3-") {
            "TDX"
        } else {
            "SEV"
        }
    }
}

/// TEE backend that deploys containers on GCP Confidential Space.
pub struct GcpConfidentialSpaceBackend {
    pub config: GcpConfig,
    auth: OnceCell<std::sync::Arc<dyn gcp_auth::TokenProvider>>,
    http: reqwest::Client,
}

impl GcpConfidentialSpaceBackend {
    pub fn new(config: GcpConfig) -> Self {
        Self {
            config,
            auth: OnceCell::new(),
            http: reqwest::Client::new(),
        }
    }

    /// Lazily initialize the GCP auth provider.
    async fn auth(&self) -> Result<&dyn gcp_auth::TokenProvider> {
        let provider = self
            .auth
            .get_or_try_init(|| async {
                gcp_auth::provider()
                    .await
                    .map_err(|e| SandboxError::CloudProvider(format!("GCP auth init: {e}")))
            })
            .await?;
        Ok(provider.as_ref())
    }

    /// Get a bearer token for the Compute Engine API.
    async fn bearer_token(&self) -> Result<String> {
        let auth = self.auth().await?;
        let token = auth
            .token(&["https://www.googleapis.com/auth/compute"])
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("GCP token: {e}")))?;
        Ok(token.as_str().to_string())
    }

    /// Base URL for instances in this project/zone.
    fn instances_url(&self) -> String {
        format!(
            "{}/projects/{}/zones/{}/instances",
            COMPUTE_BASE, self.config.project_id, self.config.zone
        )
    }

    /// Build the Compute Engine VM creation JSON body for a Confidential Space VM.
    fn build_instance_body(&self, params: &TeeDeployParams) -> serde_json::Value {
        let instance_name = format!("tee-sandbox-{}", params.sandbox_id);

        // Build metadata items. Confidential Space reads these at boot:
        // - tee-image-reference: container to run
        // - tee-env-*: environment variables injected into the container
        let mut metadata_items = vec![
            serde_json::json!({
                "key": "tee-image-reference",
                "value": params.image
            }),
            serde_json::json!({
                "key": "tee-container-log-redirect",
                "value": "true"
            }),
            serde_json::json!({
                "key": "tee-restart-policy",
                "value": "Never"
            }),
        ];

        for (key, value) in &params.env_vars {
            metadata_items.push(serde_json::json!({
                "key": format!("tee-env-{key}"),
                "value": value
            }));
        }

        let mut body = serde_json::json!({
            "name": instance_name,
            "machineType": format!(
                "zones/{}/machineTypes/{}",
                self.config.zone, self.config.machine_type
            ),
            "confidentialInstanceConfig": {
                "confidentialInstanceType": self.config.confidential_instance_type(),
                "enableConfidentialCompute": true
            },
            "scheduling": {
                "automaticRestart": true,
                "onHostMaintenance": "TERMINATE"
            },
            "shieldedInstanceConfig": {
                "enableSecureBoot": true,
                "enableVtpm": true,
                "enableIntegrityMonitoring": true
            },
            "disks": [{
                "boot": true,
                "autoDelete": true,
                "initializeParams": {
                    "sourceImage": format!(
                        "projects/confidential-space-images/global/images/family/{}",
                        self.config.confidential_space_image
                    )
                }
            }],
            "networkInterfaces": [{
                "nicType": "GVNIC",
                "accessConfigs": [{
                    "type": "ONE_TO_ONE_NAT",
                    "name": "External NAT"
                }]
            }],
            "metadata": {
                "items": metadata_items
            }
        });

        // Optional: service account for GCP API access from inside the VM.
        if let Some(ref sa_email) = self.config.service_account_email {
            body["serviceAccounts"] = serde_json::json!([{
                "email": sa_email,
                "scopes": ["https://www.googleapis.com/auth/cloud-platform"]
            }]);
        }

        // Optional: custom network/subnet.
        if let Some(ref network) = self.config.network {
            body["networkInterfaces"][0]["network"] = serde_json::json!(network);
        }
        if let Some(ref subnet) = self.config.subnet {
            body["networkInterfaces"][0]["subnetwork"] = serde_json::json!(subnet);
        }

        body
    }

    /// Poll Compute Engine until the instance is RUNNING, then return its external IP.
    async fn wait_for_running(&self, instance_name: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(SandboxError::CloudProvider(format!(
                    "GCP instance {instance_name} did not reach RUNNING state within timeout"
                )));
            }

            let token = self.bearer_token().await?;
            let url = format!("{}/{}", self.instances_url(), instance_name);
            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| SandboxError::CloudProvider(format!("Get instance: {e}")))?;

            if resp.status().is_success() {
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| SandboxError::CloudProvider(format!("Parse response: {e}")))?;

                let status = body["status"].as_str().unwrap_or("");
                if status == "RUNNING" {
                    let ip = body["networkInterfaces"][0]["accessConfigs"][0]["natIP"]
                        .as_str()
                        .ok_or_else(|| {
                            SandboxError::CloudProvider("No external IP assigned".into())
                        })?
                        .to_string();
                    return Ok(ip);
                }
                if status == "TERMINATED" || status == "SUSPENDED" {
                    return Err(SandboxError::CloudProvider(format!(
                        "GCP instance {instance_name} entered terminal state: {status}"
                    )));
                }
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

#[async_trait::async_trait]
impl TeeBackend for GcpConfidentialSpaceBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let token = self.bearer_token().await?;
        let instance_name = format!("tee-sandbox-{}", params.sandbox_id);
        let body = self.build_instance_body(params);

        // Create the Confidential Space VM.
        let resp = self
            .http
            .post(&self.instances_url())
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Create instance: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "GCP instance creation failed: {err_body}"
            )));
        }

        // Wait for RUNNING and get external IP.
        let public_ip = self.wait_for_running(&instance_name).await?;
        let sidecar_url = format!("http://{}:{}", public_ip, params.http_port);

        // Wait for sidecar health inside the confidential VM.
        super::wait_for_sidecar_health(
            &sidecar_url,
            &params.sidecar_token,
            Duration::from_secs(300),
        )
        .await?;

        // Fetch attestation from the sidecar (which reads from teeserver.sock).
        let attestation =
            super::fetch_sidecar_attestation(&sidecar_url, &params.sidecar_token).await?;

        let metadata = serde_json::json!({
            "gcp_project": self.config.project_id,
            "gcp_zone": self.config.zone,
            "gcp_instance_name": instance_name,
            "public_ip": public_ip,
            "machine_type": self.config.machine_type,
        });

        Ok(TeeDeployment {
            deployment_id: instance_name,
            sidecar_url,
            ssh_port: params.ssh_port,
            attestation,
            metadata_json: metadata.to_string(),
        })
    }

    async fn attestation(&self, deployment_id: &str) -> Result<AttestationReport> {
        let (sidecar_url, token) = super::sidecar_info_for_deployment(deployment_id)?;
        super::fetch_sidecar_attestation(&sidecar_url, &token).await
    }

    async fn stop(&self, deployment_id: &str) -> Result<()> {
        let token = self.bearer_token().await?;
        let url = format!("{}/{}/stop", self.instances_url(), deployment_id);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Stop instance: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "GCP stop failed: {err_body}"
            )));
        }
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        let token = self.bearer_token().await?;
        let url = format!("{}/{}", self.instances_url(), deployment_id);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Delete instance: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "GCP delete failed: {err_body}"
            )));
        }
        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        self.config.inferred_tee_type()
    }

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

fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| {
        SandboxError::Validation(format!(
            "GCP Confidential Space backend requires {name} environment variable"
        ))
    })
}
