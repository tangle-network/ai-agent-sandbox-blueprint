//! AzureSkrBackend implementation helpers (auth, network, CVM lifecycle).

use super::*;

impl AzureSkrBackend {
    pub fn new(config: AzureConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            token_cache: RwLock::new(None),
        }
    }

    /// Get an Azure Resource Manager bearer token, refreshing if expired.
    ///
    /// Single-flighted: the refresh holds the write lock across the network
    /// round-trip and double-checks the cache after acquiring it, so N concurrent
    /// callers (deploy fans out many ARM calls) trigger at most one OAuth POST —
    /// the losers of the lock race observe the token the winner just cached.
    pub(crate) async fn arm_token(&self) -> Result<String> {
        // Fast path: a valid cached token under a shared read lock.
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache
                && cached.expires_at > std::time::Instant::now() + Duration::from_secs(60)
            {
                return Ok(cached.token.clone());
            }
        }

        // Slow path: serialize refreshes on the write lock and re-check, so only
        // the first arrival performs the network fetch.
        let mut cache = self.token_cache.write().await;
        if let Some(ref cached) = *cache
            && cached.expires_at > std::time::Instant::now() + Duration::from_secs(60)
        {
            return Ok(cached.token.clone());
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

        // Cache the token under the write lock we already hold (single-flight).
        *cache = Some(CachedToken {
            token: access_token.clone(),
            expires_at: std::time::Instant::now() + Duration::from_secs(expires_in),
        });

        Ok(access_token)
    }

    pub(crate) fn compute_base_url(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Compute",
            self.config.subscription_id, self.config.resource_group
        )
    }

    pub(crate) fn network_base_url(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Network",
            self.config.subscription_id, self.config.resource_group
        )
    }

    /// Create a public IP address for a VM.
    pub(crate) async fn create_public_ip(&self, name: &str) -> Result<String> {
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
    pub(crate) async fn create_nic(&self, name: &str, public_ip_id: &str) -> Result<String> {
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
    pub(crate) fn build_cvm_body(
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
        let image_ref: serde_json::Value = serde_json::from_str(&self.config.vm_image)
            .unwrap_or_else(|_| {
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
    pub(crate) async fn wait_for_running(&self, vm_name: &str, pip_name: &str) -> Result<String> {
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
                    // Get the public IP address, reusing the token fetched for
                    // the VM GET this iteration (avoids a redundant OAuth call).
                    let pip_url = format!(
                        "{}/publicIPAddresses/{}?api-version={}",
                        self.network_base_url(),
                        pip_name,
                        NETWORK_API_VERSION
                    );
                    let pip_resp = self
                        .http
                        .get(&pip_url)
                        .bearer_auth(&token)
                        .send()
                        .await
                        .map_err(|e| SandboxError::CloudProvider(format!("Get public IP: {e}")))?;

                    if pip_resp.status().is_success() {
                        let pip_body: serde_json::Value = pip_resp.json().await.map_err(|e| {
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
    pub(crate) async fn delete_network_resource(&self, resource_url: &str) -> Result<()> {
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
