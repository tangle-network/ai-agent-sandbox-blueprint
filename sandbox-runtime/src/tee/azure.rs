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
struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

/// TEE backend that deploys containers on Azure Confidential VMs with SKR.
pub struct AzureSkrBackend {
    pub config: AzureConfig,
    http: reqwest::Client,
    token_cache: RwLock<Option<CachedToken>>,
}

impl AzureSkrBackend {
    pub fn new(config: AzureConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            token_cache: RwLock::new(None),
        }
    }

    /// Get an Azure Resource Manager bearer token, refreshing if expired.
    async fn arm_token(&self) -> Result<String> {
        // Check cache first.
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache {
                if cached.expires_at > std::time::Instant::now() + Duration::from_secs(60) {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Fetch new token via OAuth2 client credentials.
        let token_url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.config.tenant_id
        );

        let resp = self
            .http
            .post(&token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("scope", "https://management.azure.com/.default"),
            ])
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Azure token request: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "Azure token request failed: {err_body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Azure token parse: {e}")))?;

        let access_token = body["access_token"]
            .as_str()
            .ok_or_else(|| {
                SandboxError::CloudProvider("No access_token in Azure token response".into())
            })?
            .to_string();

        let expires_in = body["expires_in"]
            .as_u64()
            .or_else(|| body["expires_in"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(3600);

        // Cache the token.
        let mut cache = self.token_cache.write().await;
        *cache = Some(CachedToken {
            token: access_token.clone(),
            expires_at: std::time::Instant::now() + Duration::from_secs(expires_in),
        });

        Ok(access_token)
    }

    fn compute_base_url(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Compute",
            self.config.subscription_id, self.config.resource_group
        )
    }

    fn network_base_url(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Network",
            self.config.subscription_id, self.config.resource_group
        )
    }

    /// Create a public IP address for a VM.
    async fn create_public_ip(&self, name: &str) -> Result<String> {
        let token = self.arm_token().await?;
        let url = format!(
            "{}/publicIPAddresses/{}?api-version={}",
            self.network_base_url(),
            name,
            NETWORK_API_VERSION
        );

        let body = serde_json::json!({
            "location": self.config.location,
            "properties": {
                "publicIPAllocationMethod": "Static",
                "publicIPAddressVersion": "IPv4"
            },
            "sku": {
                "name": "Standard"
            }
        });

        let resp = self
            .http
            .put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Create public IP: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "Azure public IP creation failed: {err_body}"
            )));
        }

        // Return the resource ID.
        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Parse public IP: {e}")))?;
        result["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| SandboxError::CloudProvider("No ID in public IP response".into()))
    }

    /// Create a NIC attached to the configured subnet with a public IP.
    async fn create_nic(&self, name: &str, public_ip_id: &str) -> Result<String> {
        let token = self.arm_token().await?;
        let url = format!(
            "{}/networkInterfaces/{}?api-version={}",
            self.network_base_url(),
            name,
            NETWORK_API_VERSION
        );

        let body = serde_json::json!({
            "location": self.config.location,
            "properties": {
                "ipConfigurations": [{
                    "name": "ipconfig1",
                    "properties": {
                        "subnet": {
                            "id": self.config.subnet_id
                        },
                        "publicIPAddress": {
                            "id": public_ip_id
                        },
                        "privateIPAllocationMethod": "Dynamic"
                    }
                }]
            }
        });

        let resp = self
            .http
            .put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Create NIC: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "Azure NIC creation failed: {err_body}"
            )));
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Parse NIC: {e}")))?;
        result["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| SandboxError::CloudProvider("No ID in NIC response".into()))
    }

    /// Build the Confidential VM creation JSON body.
    fn build_cvm_body(
        &self,
        params: &TeeDeployParams,
        vm_name: &str,
        nic_id: &str,
    ) -> serde_json::Value {
        // Build cloud-init custom data to start the sidecar.
        let env_obj: serde_json::Map<String, serde_json::Value> = params
            .env_vars
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        let env_json = serde_json::to_string(&env_obj).unwrap_or_default();
        let custom_data_script = format!(
            "#!/bin/bash\nset -ex\n\
             echo '{}' > /opt/sidecar/env.json\n\
             systemctl start sidecar\n",
            env_json.replace('\'', "'\\''")
        );
        let custom_data_b64 =
            base64::engine::general_purpose::STANDARD.encode(custom_data_script.as_bytes());

        // Parse image reference. Accepts JSON object format:
        // {"publisher":"...", "offer":"...", "sku":"...", "version":"..."}
        let image_ref: serde_json::Value =
            serde_json::from_str(&self.config.vm_image).unwrap_or_else(|_| {
                // Fallback: treat as a custom image ID.
                serde_json::json!({ "id": self.config.vm_image })
            });

        serde_json::json!({
            "location": self.config.location,
            "identity": { "type": "SystemAssigned" },
            "properties": {
                "hardwareProfile": { "vmSize": self.config.vm_size },
                "storageProfile": {
                    "imageReference": image_ref,
                    "osDisk": {
                        "createOption": "FromImage",
                        "managedDisk": {
                            "storageAccountType": "Premium_LRS",
                            "securityProfile": {
                                "securityEncryptionType": "VMGuestStateOnly"
                            }
                        }
                    }
                },
                "osProfile": {
                    "computerName": vm_name,
                    "adminUsername": "azureuser",
                    "customData": custom_data_b64,
                    "linuxConfiguration": {
                        "disablePasswordAuthentication": true,
                        "ssh": {
                            "publicKeys": []
                        }
                    }
                },
                "securityProfile": {
                    "securityType": "ConfidentialVM",
                    "uefiSettings": {
                        "secureBootEnabled": true,
                        "vTpmEnabled": true
                    }
                },
                "networkProfile": {
                    "networkInterfaces": [{
                        "id": nic_id,
                        "properties": { "primary": true }
                    }]
                }
            }
        })
    }

    /// Poll until the VM is running, then retrieve its public IP.
    async fn wait_for_running(&self, vm_name: &str, pip_name: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);

        // First wait for the VM to be provisioned.
        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(SandboxError::CloudProvider(format!(
                    "Azure VM {vm_name} did not reach running state within timeout"
                )));
            }

            let token = self.arm_token().await?;
            let url = format!(
                "{}/virtualMachines/{}?api-version={}&$expand=instanceView",
                self.compute_base_url(),
                vm_name,
                COMPUTE_API_VERSION
            );

            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| SandboxError::CloudProvider(format!("Get VM: {e}")))?;

            if resp.status().is_success() {
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| SandboxError::CloudProvider(format!("Parse VM: {e}")))?;

                // Check provisioning state.
                let prov_state = body["properties"]["provisioningState"]
                    .as_str()
                    .unwrap_or("");

                if prov_state == "Succeeded" {
                    // Get the public IP address.
                    let pip_url = format!(
                        "{}/publicIPAddresses/{}?api-version={}",
                        self.network_base_url(),
                        pip_name,
                        NETWORK_API_VERSION
                    );
                    let pip_token = self.arm_token().await?;
                    let pip_resp = self
                        .http
                        .get(&pip_url)
                        .bearer_auth(&pip_token)
                        .send()
                        .await
                        .map_err(|e| {
                            SandboxError::CloudProvider(format!("Get public IP: {e}"))
                        })?;

                    if pip_resp.status().is_success() {
                        let pip_body: serde_json::Value = pip_resp
                            .json()
                            .await
                            .map_err(|e| {
                                SandboxError::CloudProvider(format!("Parse public IP: {e}"))
                            })?;

                        if let Some(ip) = pip_body["properties"]["ipAddress"].as_str() {
                            return Ok(ip.to_string());
                        }
                    }
                }

                if prov_state == "Failed" {
                    return Err(SandboxError::CloudProvider(format!(
                        "Azure VM {vm_name} provisioning failed"
                    )));
                }
            }

            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    /// Delete a network resource (NIC or public IP) by its full ARM ID.
    async fn delete_network_resource(&self, resource_url: &str) -> Result<()> {
        let token = self.arm_token().await?;
        let _ = self
            .http
            .delete(resource_url)
            .bearer_auth(&token)
            .send()
            .await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TeeBackend for AzureSkrBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let vm_name = format!("tee-sandbox-{}", params.sandbox_id);
        let pip_name = format!("{}-pip", vm_name);
        let nic_name = format!("{}-nic", vm_name);

        // Create networking resources.
        let pip_id = self.create_public_ip(&pip_name).await?;
        let nic_id = self.create_nic(&nic_name, &pip_id).await?;

        // Create the Confidential VM.
        let token = self.arm_token().await?;
        let vm_url = format!(
            "{}/virtualMachines/{}?api-version={}",
            self.compute_base_url(),
            vm_name,
            COMPUTE_API_VERSION
        );
        let body = self.build_cvm_body(params, &vm_name, &nic_id);

        let resp = self
            .http
            .put(&vm_url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Create VM: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            // Clean up networking resources on failure.
            let _ = self
                .delete_network_resource(&format!(
                    "{}/networkInterfaces/{}?api-version={}",
                    self.network_base_url(),
                    nic_name,
                    NETWORK_API_VERSION
                ))
                .await;
            let _ = self
                .delete_network_resource(&format!(
                    "{}/publicIPAddresses/{}?api-version={}",
                    self.network_base_url(),
                    pip_name,
                    NETWORK_API_VERSION
                ))
                .await;
            return Err(SandboxError::CloudProvider(format!(
                "Azure VM creation failed: {err_body}"
            )));
        }

        // Wait for VM provisioning and get public IP.
        let public_ip = self.wait_for_running(&vm_name, &pip_name).await?;
        let sidecar_url = format!("http://{}:{}", public_ip, params.http_port);

        // Wait for sidecar health.
        super::wait_for_sidecar_health(
            &sidecar_url,
            &params.sidecar_token,
            Duration::from_secs(300),
        )
        .await?;

        // Fetch attestation from the sidecar (which reads the vTPM + MAA).
        let attestation =
            super::fetch_sidecar_attestation(&sidecar_url, &params.sidecar_token).await?;

        let metadata = serde_json::json!({
            "azure_vm_name": vm_name,
            "azure_resource_group": self.config.resource_group,
            "azure_location": self.config.location,
            "azure_pip_name": pip_name,
            "azure_nic_name": nic_name,
            "public_ip": public_ip,
            "vm_size": self.config.vm_size,
        });

        Ok(TeeDeployment {
            deployment_id: vm_name,
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
        let token = self.arm_token().await?;
        let url = format!(
            "{}/virtualMachines/{}/deallocate?api-version={}",
            self.compute_base_url(),
            deployment_id,
            COMPUTE_API_VERSION
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Deallocate VM: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "Azure deallocate failed: {err_body}"
            )));
        }
        Ok(())
    }

    async fn destroy(&self, deployment_id: &str) -> Result<()> {
        // Look up associated resources from metadata.
        let (nic_name, pip_name) = {
            let store = crate::runtime::sandboxes()?;
            let record = store
                .find(|r| r.tee_deployment_id.as_deref() == Some(deployment_id))?;
            if let Some(record) = record {
                if let Some(ref meta_json) = record.tee_metadata_json {
                    let meta: serde_json::Value =
                        serde_json::from_str(meta_json).unwrap_or_default();
                    (
                        meta["azure_nic_name"].as_str().map(|s| s.to_string()),
                        meta["azure_pip_name"].as_str().map(|s| s.to_string()),
                    )
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        };

        let token = self.arm_token().await?;

        // Delete the VM.
        let vm_url = format!(
            "{}/virtualMachines/{}?api-version={}",
            self.compute_base_url(),
            deployment_id,
            COMPUTE_API_VERSION
        );
        let resp = self
            .http
            .delete(&vm_url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SandboxError::CloudProvider(format!("Delete VM: {e}")))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(SandboxError::CloudProvider(format!(
                "Azure VM delete failed: {err_body}"
            )));
        }

        // Wait briefly for VM deletion to propagate before cleaning up NIC/IP.
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Clean up NIC.
        if let Some(nic) = nic_name {
            let _ = self
                .delete_network_resource(&format!(
                    "{}/networkInterfaces/{}?api-version={}",
                    self.network_base_url(),
                    nic,
                    NETWORK_API_VERSION
                ))
                .await;
        }

        // Clean up public IP.
        if let Some(pip) = pip_name {
            let _ = self
                .delete_network_resource(&format!(
                    "{}/publicIPAddresses/{}?api-version={}",
                    self.network_base_url(),
                    pip,
                    NETWORK_API_VERSION
                ))
                .await;
        }

        Ok(())
    }

    fn tee_type(&self) -> TeeType {
        TeeType::Sev
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
            "Azure SKR backend requires {name} environment variable"
        ))
    })
}
