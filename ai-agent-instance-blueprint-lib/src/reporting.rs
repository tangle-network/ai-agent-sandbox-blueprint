use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::contexts::tangle::TangleClient;
use blueprint_sdk::{info, warn};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingProvisionReport {
    pub service_id: u64,
    pub sandbox_id: String,
    pub sidecar_url: String,
    pub ssh_port: u32,
    pub tee_attestation_json: String,
    pub last_error: String,
    pub updated_at: u64,
}

static PENDING_PROVISION_REPORTS: OnceCell<crate::store::PersistentStore<PendingProvisionReport>> =
    OnceCell::new();

fn pending_reports()
-> Result<&'static crate::store::PersistentStore<PendingProvisionReport>, String> {
    PENDING_PROVISION_REPORTS
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("pending-provision-reports.json");
            crate::store::PersistentStore::open(path).map_err(|e| e.to_string())
        })
        .map_err(|e: String| e)
}

fn pending_key(service_id: u64) -> String {
    service_id.to_string()
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl PendingProvisionReport {
    fn from_output(service_id: u64, output: &ProvisionOutput, last_error: String) -> Self {
        Self {
            service_id,
            sandbox_id: output.sandbox_id.to_string(),
            sidecar_url: output.sidecar_url.to_string(),
            ssh_port: output.ssh_port,
            tee_attestation_json: output.tee_attestation_json.to_string(),
            last_error,
            updated_at: now_ts(),
        }
    }

    fn to_output(&self) -> ProvisionOutput {
        ProvisionOutput {
            sandbox_id: self.sandbox_id.clone(),
            sidecar_url: self.sidecar_url.clone(),
            ssh_port: self.ssh_port,
            tee_attestation_json: self.tee_attestation_json.clone(),
            tee_public_key_json: String::new(),
        }
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

/// Persist a pending direct-report payload for retry.
pub fn mark_pending_provision_report(
    service_id: u64,
    output: &ProvisionOutput,
    err: &str,
) -> Result<(), String> {
    let pending = PendingProvisionReport::from_output(service_id, output, err.to_string());
    pending_reports()?
        .insert(pending_key(service_id), pending)
        .map_err(|e| e.to_string())
}

/// Load pending direct-report payload, if any.
pub fn get_pending_provision_report(
    service_id: u64,
) -> Result<Option<PendingProvisionReport>, String> {
    pending_reports()?
        .get(&pending_key(service_id))
        .map_err(|e| e.to_string())
}

/// Clear pending direct-report payload.
pub fn clear_pending_provision_report(service_id: u64) -> Result<(), String> {
    pending_reports()?
        .remove(&pending_key(service_id))
        .map_err(|e| e.to_string())?;
    Ok(())
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
        clear_pending_provision_report(service_id)?;
        return Ok(());
    }

    warn!(
        service_id,
        operator = %client.account(),
        sandbox_id = %record.id,
        "Local sandbox exists but operator is not provisioned on-chain; reporting now"
    );

    let output = provision_output_from_record(record);
    match report_local_provision(client, service_id, &output).await {
        Ok(()) => {
            clear_pending_provision_report(service_id)?;
            Ok(())
        }
        Err(err) => {
            mark_pending_provision_report(service_id, &output, &err)?;
            Err(err)
        }
    }
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

/// Retry a pending direct-report payload once.
///
/// Returns `Ok(true)` when a pending record was found and processed.
pub async fn retry_pending_provision_report_once(
    client: &TangleClient,
    service_id: u64,
) -> Result<bool, String> {
    let Some(pending) = get_pending_provision_report(service_id)? else {
        return Ok(false);
    };

    if is_local_operator_provisioned(client, service_id).await? {
        clear_pending_provision_report(service_id)?;
        info!(
            service_id,
            operator = %client.account(),
            sandbox_id = %pending.sandbox_id,
            "Cleared pending provision report (already provisioned on-chain)"
        );
        return Ok(true);
    }

    let output = pending.to_output();
    match report_local_provision(client, service_id, &output).await {
        Ok(()) => {
            clear_pending_provision_report(service_id)?;
            info!(
                service_id,
                operator = %client.account(),
                sandbox_id = %output.sandbox_id,
                "Retried pending provision report successfully"
            );
            Ok(true)
        }
        Err(err) => {
            mark_pending_provision_report(service_id, &output, &err)?;
            Err(err)
        }
    }
}

/// Spawn a background worker to retry pending direct-report payloads.
pub fn spawn_pending_provision_report_worker(
    client: TangleClient,
    service_id: u64,
    mut shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    let interval_secs = std::env::var("PENDING_REPORT_RETRY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(err) = retry_pending_provision_report_once(&client, service_id).await {
                        warn!(
                            service_id,
                            error = %err,
                            "Pending provision report retry failed"
                        );
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Pending provision report worker shutting down");
                    break;
                }
            }
        }
    })
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

    #[test]
    fn pending_provision_report_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("BLUEPRINT_STATE_DIR", dir.path());
        }

        let output = ProvisionOutput {
            sandbox_id: "sb-3".to_string(),
            sidecar_url: "http://127.0.0.1:7070".to_string(),
            ssh_port: 22,
            tee_attestation_json: "{}".to_string(),
            tee_public_key_json: String::new(),
        };

        mark_pending_provision_report(42, &output, "tx failed").expect("mark");
        let pending = get_pending_provision_report(42)
            .expect("get")
            .expect("exists");
        assert_eq!(pending.service_id, 42);
        assert_eq!(pending.sandbox_id, "sb-3");
        assert_eq!(pending.ssh_port, 22);

        clear_pending_provision_report(42).expect("clear");
        assert!(
            get_pending_provision_report(42)
                .expect("get after clear")
                .is_none()
        );

        unsafe {
            std::env::remove_var("BLUEPRINT_STATE_DIR");
        }
    }
}
