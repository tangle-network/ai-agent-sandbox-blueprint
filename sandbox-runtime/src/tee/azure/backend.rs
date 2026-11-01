//! `TeeBackend` contract for Azure Confidential VM + SKR.

use super::*;

impl AzureSkrBackend {
    async fn abandon(&self, vm_name: &str, nic_name: &str, pip_name: &str) {
        let Ok(token) = self.arm_token().await else {
            tracing::warn!(
                vm_name,
                "failed to delete the Azure VM after deploy was refused"
            );
            return;
        };
        let vm_url = format!(
            "{}/virtualMachines/{}?api-version={}",
            self.compute_base_url(),
            vm_name,
            COMPUTE_API_VERSION
        );
        match self.http.delete(&vm_url).bearer_auth(&token).send().await {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    vm_name,
                    "failed to delete the Azure VM after deploy was refused"
                );
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    vm_name,
                    "failed to delete the Azure VM after deploy was refused"
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
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
    }

    async fn or_abandon<T>(
        &self,
        vm_name: &str,
        nic_name: &str,
        pip_name: &str,
        result: Result<T>,
    ) -> Result<T> {
        if result.is_err() {
            self.abandon(vm_name, nic_name, pip_name).await;
        }
        result
    }
}

#[async_trait::async_trait]
impl TeeBackend for AzureSkrBackend {
    async fn deploy(&self, params: &TeeDeployParams) -> Result<TeeDeployment> {
        let vm_name = format!("tee-sandbox-{}", params.sandbox_id);
        let pip_name = format!("{vm_name}-pip");
        let nic_name = format!("{vm_name}-nic");

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
            // The VM was not created. Drop the NIC and public IP only.
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

        // The VM exists. Every later failure deletes the VM, NIC, and public IP.
        let running = self.wait_for_running(&vm_name, &pip_name).await;
        let public_ip = self
            .or_abandon(&vm_name, &nic_name, &pip_name, running)
            .await?;
        let sidecar_url = format!("http://{}:{}", public_ip, params.http_port);

        let healthy = super::wait_for_sidecar_health(
            &sidecar_url,
            &params.sidecar_token,
            Duration::from_secs(300),
        )
        .await;
        self.or_abandon(&vm_name, &nic_name, &pip_name, healthy)
            .await?;

        let fetched = super::fetch_sidecar_attestation_with_report_data(
            &sidecar_url,
            &params.sidecar_token,
            params.attestation_report_data,
        )
        .await;
        let attestation = self
            .or_abandon(&vm_name, &nic_name, &pip_name, fetched)
            .await?;
        let verified = super::require_production_attestation(
            &attestation,
            params.attestation_report_data.as_ref(),
        )
        .await;
        self.or_abandon(&vm_name, &nic_name, &pip_name, verified)
            .await?;

        let metadata = serde_json::json!({
            "azure_vm_name": vm_name,
            "azure_resource_group": self.config.resource_group,
            "azure_location": self.config.location,
            "azure_pip_name": pip_name,
            "azure_nic_name": nic_name,
            "public_ip": public_ip,
            "vm_size": self.config.vm_size,
        });

        if !params.extra_ports.is_empty() {
            tracing::warn!("Extra ports not yet supported for Azure backend — ignored");
        }

        Ok(TeeDeployment {
            deployment_id: vm_name,
            sidecar_url,
            ssh_port: params.ssh_port,
            attestation,
            metadata_json: metadata.to_string(),
            extra_ports: std::collections::HashMap::new(),
        })
    }

    async fn attestation(
        &self,
        deployment_id: &str,
        report_data: Option<[u8; 64]>,
    ) -> Result<AttestationReport> {
        let (sidecar_url, token) = super::sidecar_info_for_deployment(deployment_id)?;
        let attestation =
            super::fetch_sidecar_attestation_with_report_data(&sidecar_url, &token, report_data)
                .await?;
        super::require_production_attestation(&attestation, report_data.as_ref()).await?;
        Ok(attestation)
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
            let record = store.find(|r| r.tee_deployment_id.as_deref() == Some(deployment_id))?;
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

    fn supports_attestation_report_data(&self) -> bool {
        true
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
