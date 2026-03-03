use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::contexts::tangle::TangleClient;
use blueprint_sdk::{info, warn};

use crate::{ProvisionOutput, SandboxRecord};

sol! {
    #[sol(rpc)]
    interface IInstanceLifecycleReporter {
        function reportProvisioned(
            uint64 serviceId,
            string sandboxId,
            string sidecarUrl,
            uint32 sshPort,
            string teeAttestationJson
        ) external;

        function reportDeprovisioned(uint64 serviceId) external;
    }
}

sol! {
    #[sol(rpc)]
    interface IInstanceLifecycleReader {
        function isOperatorProvisioned(uint64 serviceId, address operator) external view returns (bool);
    }
}

/// Build a direct-report payload from the locally persisted sandbox record.
///
/// This supports idempotent startup reconciliation when the sandbox already exists locally.
pub fn provision_output_from_record(record: &SandboxRecord) -> ProvisionOutput {
    ProvisionOutput {
        sandbox_id: record.id.clone(),
        sidecar_url: record.sidecar_url.clone(),
        ssh_port: record.ssh_port.unwrap_or(0) as u32,
        tee_attestation_json: record.tee_attestation_json.clone().unwrap_or_default(),
        // Not consumed by on-chain lifecycle reporting; preserved for API compatibility.
        tee_public_key_json: String::new(),
    }
}

/// Check whether the local operator account is already marked provisioned on-chain.
pub async fn is_local_operator_provisioned(
    client: &TangleClient,
    service_id: u64,
) -> Result<bool, String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| {
            format!("Failed to resolve blueprint manager for service {service_id}: {err}")
        })?
        .ok_or_else(|| format!("No blueprint manager found for service {service_id}"))?;

    let operator = client.account();
    let contract = IInstanceLifecycleReader::new(manager, client.provider().clone());
    let provisioned = contract
        .isOperatorProvisioned(service_id, operator)
        .call()
        .await
        .map_err(|err| format!("isOperatorProvisioned call failed: {err}"))?;

    Ok(provisioned)
}

/// Reconcile local instance state with on-chain provision status.
///
/// If local state exists but the operator is not provisioned on-chain, this function
/// sends `reportProvisioned` using local record data.
pub async fn ensure_local_provision_reported(
    client: &TangleClient,
    service_id: u64,
    record: &SandboxRecord,
) -> Result<(), String> {
    if is_local_operator_provisioned(client, service_id).await? {
        info!(
            service_id,
            operator = %client.account(),
            sandbox_id = %record.id,
            "Local sandbox already marked provisioned on-chain"
        );
        return Ok(());
    }

    warn!(
        service_id,
        operator = %client.account(),
        sandbox_id = %record.id,
        "Local sandbox exists but operator is not provisioned on-chain; reporting now"
    );

    let output = provision_output_from_record(record);
    report_local_provision(client, service_id, &output).await
}

/// Report local provision state directly to the blueprint manager contract.
///
/// This is the canonical instance lifecycle sync path.
pub async fn report_local_provision(
    client: &TangleClient,
    service_id: u64,
    output: &ProvisionOutput,
) -> Result<(), String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| {
            format!("Failed to resolve blueprint manager for service {service_id}: {err}")
        })?
        .ok_or_else(|| format!("No blueprint manager found for service {service_id}"))?;

    let wallet = client
        .wallet()
        .map_err(|err| format!("Failed to load operator wallet: {err}"))?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect(client.config.http_rpc_endpoint.as_str())
        .await
        .map_err(|err| format!("Failed to connect signer provider: {err}"))?;

    let contract = IInstanceLifecycleReporter::new(manager, provider);
    let pending_tx = contract
        .reportProvisioned(
            service_id,
            output.sandbox_id.to_string(),
            output.sidecar_url.to_string(),
            output.ssh_port,
            output.tee_attestation_json.to_string(),
        )
        .send()
        .await
        .map_err(|err| format!("reportProvisioned transaction failed: {err}"))?;

    let receipt = pending_tx
        .get_receipt()
        .await
        .map_err(|err| format!("reportProvisioned receipt fetch failed: {err}"))?;
    if !receipt.status() {
        return Err("reportProvisioned transaction reverted".to_string());
    }

    info!(
        service_id,
        tx_hash = %receipt.transaction_hash,
        sandbox_id = %output.sandbox_id,
        "Instance provision reported on-chain via direct manager call"
    );
    Ok(())
}

/// Report local deprovision state directly to the blueprint manager contract.
pub async fn report_local_deprovision(
    client: &TangleClient,
    service_id: u64,
) -> Result<(), String> {
    let manager = client
        .get_blueprint_manager(service_id)
        .await
        .map_err(|err| {
            format!("Failed to resolve blueprint manager for service {service_id}: {err}")
        })?
        .ok_or_else(|| format!("No blueprint manager found for service {service_id}"))?;

    let wallet = client
        .wallet()
        .map_err(|err| format!("Failed to load operator wallet: {err}"))?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect(client.config.http_rpc_endpoint.as_str())
        .await
        .map_err(|err| format!("Failed to connect signer provider: {err}"))?;

    let contract = IInstanceLifecycleReporter::new(manager, provider);
    let pending_tx = contract
        .reportDeprovisioned(service_id)
        .send()
        .await
        .map_err(|err| format!("reportDeprovisioned transaction failed: {err}"))?;

    let receipt = pending_tx
        .get_receipt()
        .await
        .map_err(|err| format!("reportDeprovisioned receipt fetch failed: {err}"))?;
    if !receipt.status() {
        return Err("reportDeprovisioned transaction reverted".to_string());
    }

    info!(
        service_id,
        tx_hash = %receipt.transaction_hash,
        "Instance deprovision reported on-chain via direct manager call"
    );
    Ok(())
}

/// Best-effort wrapper that logs warning and returns success.
pub async fn try_report_local_deprovision(client: Option<&TangleClient>, service_id: u64) {
    if let Some(client) = client {
        if let Err(err) = report_local_deprovision(client, service_id).await {
            warn!(
                service_id,
                error = %err,
                "Failed to report direct deprovision to manager"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SandboxState;
    use std::collections::HashMap;

    #[test]
    fn provision_output_from_record_keeps_attestation() {
        let record = SandboxRecord {
            id: "sb-1".to_string(),
            container_id: "cid-1".to_string(),
            sidecar_url: "http://127.0.0.1:8080".to_string(),
            sidecar_port: 8080,
            ssh_port: Some(2222),
            token: "token".to_string(),
            created_at: 1,
            cpu_cores: 2,
            memory_mb: 2048,
            state: SandboxState::Running,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 1,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "img".to_string(),
            base_env_json: "{}".to_string(),
            user_env_json: "{}".to_string(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: Some("{\"quote\":\"abc\"}".to_string()),
            name: "sandbox".to_string(),
            agent_identifier: "agent".to_string(),
            metadata_json: "{}".to_string(),
            disk_gb: 20,
            stack: "default".to_string(),
            owner: "0xabc".to_string(),
            tee_config: None,
            extra_ports: HashMap::new(),
        };

        let output = provision_output_from_record(&record);
        assert_eq!(output.sandbox_id, "sb-1");
        assert_eq!(output.sidecar_url, "http://127.0.0.1:8080");
        assert_eq!(output.ssh_port, 2222);
        assert_eq!(output.tee_attestation_json, "{\"quote\":\"abc\"}");
        assert!(output.tee_public_key_json.is_empty());
    }

    #[test]
    fn provision_output_from_record_defaults_optional_fields() {
        let record = SandboxRecord {
            id: "sb-2".to_string(),
            container_id: "cid-2".to_string(),
            sidecar_url: "http://127.0.0.1:9090".to_string(),
            sidecar_port: 9090,
            ssh_port: None,
            token: "token".to_string(),
            created_at: 1,
            cpu_cores: 2,
            memory_mb: 2048,
            state: SandboxState::Running,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 1,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "img".to_string(),
            base_env_json: "{}".to_string(),
            user_env_json: "{}".to_string(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: "sandbox".to_string(),
            agent_identifier: "agent".to_string(),
            metadata_json: "{}".to_string(),
            disk_gb: 20,
            stack: "default".to_string(),
            owner: "0xabc".to_string(),
            tee_config: None,
            extra_ports: HashMap::new(),
        };

        let output = provision_output_from_record(&record);
        assert_eq!(output.ssh_port, 0);
        assert!(output.tee_attestation_json.is_empty());
    }
}
