//! Auto-provision: reads service config from BSM on-chain, provisions sandbox
//! automatically on startup.
//!
//! Flow:
//! 1. On startup, check if already provisioned (`get_instance_sandbox()`)
//! 2. If not, poll `getServiceConfig(serviceId)` from BSM via RPC
//! 3. When config available, decode as `ProvisionRequest`, call `provision_core()`
//! 4. Store sandbox record via `set_instance_sandbox()`
//! 5. Report provision directly to manager contract (`reportProvisioned`)
//!

use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol_types::SolValue;
use blueprint_sdk::{info, warn};
use std::time::Duration;

use crate::tee::TeeBackend;
use crate::{
    IBsmRead, ProvisionRequest, clear_instance_sandbox, ensure_local_provision_reported,
    get_instance_sandbox, mark_pending_provision_report, provision_core, report_local_provision,
    set_instance_sandbox,
};

/// Configuration for auto-provision from environment.
#[derive(Debug, Clone)]
pub struct AutoProvisionConfig {
    /// BSM contract address.
    pub bsm_address: Address,
    /// HTTP RPC endpoint for querying on-chain state.
    pub http_rpc_endpoint: String,
    /// Service ID for this instance.
    pub service_id: u64,
    /// How often to poll for config (seconds).
    pub poll_interval_secs: u64,
    /// Maximum number of poll attempts before giving up.
    pub max_attempts: u32,
}

impl AutoProvisionConfig {
    /// Build config from environment variables.
    ///
    /// Required: `BSM_ADDRESS`
    /// Optional: `HTTP_RPC_ENDPOINT` / `RPC_URL` (default: http://127.0.0.1:8545),
    ///           `AUTO_PROVISION_POLL_SECS` (default: 5),
    ///           `AUTO_PROVISION_MAX_ATTEMPTS` (default: 60)
    pub fn from_env(service_id: u64) -> Option<Self> {
        let bsm_str = std::env::var("BSM_ADDRESS").ok()?;
        let bsm_address: Address = bsm_str.parse().ok().or_else(|| {
            warn!("Invalid BSM_ADDRESS: {bsm_str}");
            None
        })?;

        let http_rpc_endpoint = std::env::var("HTTP_RPC_ENDPOINT")
            .or_else(|_| std::env::var("RPC_URL"))
            .unwrap_or_else(|_| "http://127.0.0.1:8545".to_string());

        let poll_interval_secs: u64 = std::env::var("AUTO_PROVISION_POLL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let max_attempts: u32 = std::env::var("AUTO_PROVISION_MAX_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        Some(Self {
            bsm_address,
            http_rpc_endpoint,
            service_id,
            poll_interval_secs,
            max_attempts,
        })
    }
}

/// Read service config from the BSM contract via RPC.
///
/// Returns the raw config bytes, or `None` if no config is stored yet.
pub async fn read_service_config(config: &AutoProvisionConfig) -> Result<Option<Vec<u8>>, String> {
    let url: url::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IBsmRead::new(config.bsm_address, &provider);

    let result = contract
        .getServiceConfig(config.service_id)
        .call()
        .await
        .map_err(|e| format!("getServiceConfig RPC failed: {e}"))?;

    let bytes = result.0;
    if bytes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(bytes.to_vec()))
    }
}

/// Read service owner from the BSM contract via RPC.
///
/// Returns the owner address as a lowercase hex string, or empty string if not set.
pub async fn read_service_owner(config: &AutoProvisionConfig) -> Result<String, String> {
    let url: url::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IBsmRead::new(config.bsm_address, &provider);

    let result = contract
        .serviceOwner(config.service_id)
        .call()
        .await
        .map_err(|e| format!("serviceOwner RPC failed: {e}"))?;

    let addr = result.0;
    if addr.is_zero() {
        Ok(String::new())
    } else {
        Ok(format!("{addr}").to_lowercase())
    }
}

/// Decode raw config bytes as a `ProvisionRequest`.
///
/// The on-chain config is stored as ABI-encoded params (flat tuple, no outer offset prefix),
/// e.g. from `cast abi-encode "f(string,...)" ...` or `abi.encode(field1, field2, ...)`.
/// Accept both params-encoded and tuple-encoded representations.
pub fn decode_provision_config(config_bytes: &[u8]) -> Result<ProvisionRequest, String> {
    ProvisionRequest::abi_decode_params(config_bytes)
        .or_else(|_| ProvisionRequest::abi_decode(config_bytes))
        .map_err(|e| format!("Failed to decode ProvisionRequest from service config: {e}"))
}

fn bind_service_id(mut record: crate::SandboxRecord, service_id: u64) -> crate::SandboxRecord {
    record.service_id = Some(service_id);
    record
}

fn should_reuse_existing_record(
    record: &crate::SandboxRecord,
    service_id: u64,
    current_owner: Option<&str>,
) -> bool {
    if record.service_id == Some(service_id) {
        return true;
    }

    record.service_id.is_none()
        && current_owner
            .map(|owner| !owner.is_empty() && record.owner.eq_ignore_ascii_case(owner))
            .unwrap_or(false)
}

fn sync_runtime_service_binding(record: &crate::SandboxRecord) -> Result<(), String> {
    let Some(service_id) = record.service_id else {
        return Ok(());
    };

    if let Ok(store) = crate::runtime::sandboxes() {
        let updated = store.update(&record.id, |existing| {
            existing.service_id = Some(service_id);
        });

        if matches!(updated, Ok(true)) {
            return Ok(());
        }

        let mut sealed = record.clone();
        crate::runtime::seal_record(&mut sealed).map_err(|e| e.to_string())?;
        store.insert(record.id.clone(), sealed)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

async fn reset_stale_instance_record(
    record: &crate::SandboxRecord,
    tee: Option<&dyn TeeBackend>,
) -> Result<(), String> {
    warn!(
        sandbox_id = %record.id,
        previous_service_id = ?record.service_id,
        previous_owner = %record.owner,
        "Auto-provision: clearing stale singleton instance state before reprovisioning"
    );

    if let Err(err) = crate::runtime::delete_sidecar(record, tee).await {
        warn!(
            sandbox_id = %record.id,
            error = %err,
            "Auto-provision: stale sandbox teardown failed; clearing local state anyway"
        );
    }

    if let Ok(store) = crate::runtime::sandboxes() {
        let _ = store.remove(&record.id);
    }

    clear_instance_sandbox().map_err(|e| e.to_string())?;
    Ok(())
}

async fn reuse_existing_instance_record(
    record: crate::SandboxRecord,
    service_id: u64,
    report_client: Option<&blueprint_sdk::contexts::tangle::TangleClient>,
) -> Result<(), String> {
    let record = bind_service_id(record, service_id);
    set_instance_sandbox(record.clone()).map_err(|e| e.to_string())?;
    sync_runtime_service_binding(&record)?;

    info!(
        "Auto-provision: local instance already provisioned (sandbox_id='{}')",
        record.id
    );

    if let Some(client) = report_client {
        if let Err(err) = ensure_local_provision_reported(client, service_id, &record).await {
            warn!(
                service_id = service_id,
                error = %err,
                sandbox_id = %record.id,
                "Auto-provision: reconcile report failed; pending report will be retried"
            );
        }
    }

    Ok(())
}

/// Run auto-provision: poll for config and provision when available.
///
/// This is designed to be spawned as a background task. It will:
/// 1. Check if already provisioned (skip if so)
/// 2. Poll `getServiceConfig` until config is available
/// 3. Decode as `ProvisionRequest` and call `provision_core`
/// 4. Store the sandbox record
pub async fn run_auto_provision(
    config: AutoProvisionConfig,
    tee: Option<&dyn TeeBackend>,
    report_client: Option<blueprint_sdk::contexts::tangle::TangleClient>,
) -> Result<(), String> {
    // Already provisioned locally?
    if let Some(record) = get_instance_sandbox().map_err(|e| e.to_string())? {
        if should_reuse_existing_record(&record, config.service_id, None) {
            return reuse_existing_instance_record(
                record,
                config.service_id,
                report_client.as_ref(),
            )
            .await;
        }

        if record.service_id.is_none() {
            let owner = read_service_owner(&config).await?;
            if should_reuse_existing_record(&record, config.service_id, Some(&owner)) {
                return reuse_existing_instance_record(
                    record,
                    config.service_id,
                    report_client.as_ref(),
                )
                .await;
            }
        }

        reset_stale_instance_record(&record, tee).await?;
    }

    info!(
        "Auto-provision: polling BSM {} for service {} config (interval={}s, max_attempts={})",
        config.bsm_address, config.service_id, config.poll_interval_secs, config.max_attempts
    );

    let mut attempts = 0;
    let config_bytes = loop {
        attempts += 1;
        match read_service_config(&config).await {
            Ok(Some(bytes)) => {
                info!(
                    "Auto-provision: service config found ({} bytes)",
                    bytes.len()
                );
                break bytes;
            }
            Ok(None) => {
                if attempts >= config.max_attempts {
                    return Err(format!(
                        "Auto-provision: no service config after {} attempts",
                        config.max_attempts
                    ));
                }
                if attempts % 12 == 1 {
                    info!(
                        "Auto-provision: waiting for service config (attempt {}/{})",
                        attempts, config.max_attempts
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Auto-provision: RPC error (attempt {}/{}): {e}",
                    attempts, config.max_attempts
                );
                if attempts >= config.max_attempts {
                    return Err(format!(
                        "Auto-provision: RPC failed after {} attempts: {e}",
                        config.max_attempts
                    ));
                }
            }
        }

        // Check if provisioned by another path.
        if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
            info!("Auto-provision: instance was provisioned externally, skipping");
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    };

    // Decode config
    let request = decode_provision_config(&config_bytes)?;
    info!(
        "Auto-provision: decoded config — name='{}', image='{}', tee={}",
        request.name, request.image, request.tee_required
    );

    // Read service owner from chain so the sandbox record has correct ownership.
    // We never auto-provision ownerless instances because instance API auth relies on owner.
    let mut owner_attempts = 0;
    let owner = loop {
        owner_attempts += 1;
        match read_service_owner(&config).await {
            Ok(addr) if !addr.is_empty() => {
                info!("Auto-provision: service owner = {addr}");
                break addr;
            }
            Ok(_) => {
                warn!(
                    "Auto-provision: service owner not set yet (attempt {}/{})",
                    owner_attempts, config.max_attempts
                );
            }
            Err(e) => {
                warn!(
                    "Auto-provision: failed to read service owner (attempt {}/{}): {e}",
                    owner_attempts, config.max_attempts
                );
            }
        }

        if owner_attempts >= config.max_attempts {
            return Err(format!(
                "Auto-provision: service owner unavailable after {} attempts",
                config.max_attempts
            ));
        }

        // Check if provisioned by another path while waiting for owner.
        if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
            info!("Auto-provision: instance was provisioned externally, skipping");
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    };

    // Final check before provisioning.
    if get_instance_sandbox().map_err(|e| e.to_string())?.is_some() {
        info!("Auto-provision: instance was provisioned externally, skipping");
        return Ok(());
    }

    // Provision
    let (output, record) = provision_core(&request, tee, &owner).await?;
    let record = bind_service_id(record, config.service_id);

    // Store record
    set_instance_sandbox(record.clone()).map_err(|e| e.to_string())?;
    sync_runtime_service_binding(&record)?;

    if let Some(client) = report_client.as_ref() {
        if let Err(err) = report_local_provision(client, config.service_id, &output).await {
            warn!(
                service_id = config.service_id,
                error = %err,
                sandbox_id = %output.sandbox_id,
                "Auto-provision: direct report failed; queued pending report for retry"
            );
            mark_pending_provision_report(config.service_id, &output, &err)?;
        }
    }

    info!(
        "Auto-provision: sandbox '{}' created at {} (ssh_port={})",
        output.sandbox_id, output.sidecar_url, output.ssh_port
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_record(service_id: Option<u64>, owner: &str) -> crate::SandboxRecord {
        crate::SandboxRecord {
            id: "sandbox-test-1".to_string(),
            container_id: "ctr-test-1".to_string(),
            sidecar_url: "http://127.0.0.1:9202".to_string(),
            sidecar_port: 9202,
            ssh_port: None,
            token: "token".to_string(),
            created_at: 1,
            cpu_cores: 2,
            memory_mb: 2048,
            state: crate::SandboxState::Running,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 1,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "agent-dev".to_string(),
            base_env_json: "{}".to_string(),
            user_env_json: "{}".to_string(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: "instance".to_string(),
            agent_identifier: "test-agent".to_string(),
            metadata_json: "{}".to_string(),
            disk_gb: 20,
            stack: "default".to_string(),
            owner: owner.to_string(),
            service_id,
            tee_config: None,
            extra_ports: HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
        }
    }

    #[test]
    fn config_from_env_returns_none_without_bsm() {
        // BSM_ADDRESS not set → None
        unsafe { std::env::remove_var("BSM_ADDRESS") };
        assert!(AutoProvisionConfig::from_env(1).is_none());
    }

    #[test]
    fn bind_service_id_sets_binding() {
        let record = bind_service_id(test_record(None, "0xabc"), 7);
        assert_eq!(record.service_id, Some(7));
    }

    #[test]
    fn reuse_check_accepts_matching_bound_service() {
        let record = test_record(Some(7), "0xabc");
        assert!(should_reuse_existing_record(&record, 7, None));
    }

    #[test]
    fn reuse_check_accepts_legacy_record_for_same_owner() {
        let record = test_record(None, "0xabc");
        assert!(should_reuse_existing_record(&record, 7, Some("0xAbC")));
    }

    #[test]
    fn reuse_check_rejects_legacy_record_for_different_owner() {
        let record = test_record(None, "0xabc");
        assert!(!should_reuse_existing_record(&record, 7, Some("0xdef")));
    }

    #[test]
    fn decode_provision_config_roundtrip() {
        use blueprint_sdk::alloy::sol_types::SolValue;

        let request = ProvisionRequest {
            name: "test-sandbox".to_string(),
            image: "agent-dev".to_string(),
            stack: "default".to_string(),
            agent_identifier: "test-agent".to_string(),
            env_json: "{}".to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: true,
            ssh_public_key: "ssh-ed25519 AAAA test".to_string(),
            web_terminal_enabled: false,
            max_lifetime_seconds: 3600,
            idle_timeout_seconds: 900,
            cpu_cores: 2,
            memory_mb: 4096,
            disk_gb: 20,
            sidecar_token: String::new(),
            tee_required: false,
            tee_type: 0,
        };

        // On-chain config is stored as params encoding (flat tuple, no outer offset),
        // matching `cast abi-encode` / `abi.encode(field1, field2, ...)`.
        let encoded = request.abi_encode_params();
        let decoded = decode_provision_config(&encoded).unwrap();

        assert_eq!(decoded.name, "test-sandbox");
        assert_eq!(decoded.image, "agent-dev");
        assert_eq!(decoded.cpu_cores, 2);
        assert_eq!(decoded.memory_mb, 4096);
        assert!(decoded.ssh_enabled);
    }

    #[test]
    fn decode_provision_config_tuple_encoding() {
        use blueprint_sdk::alloy::sol_types::SolValue;

        let request = ProvisionRequest {
            name: "tuple-sandbox".to_string(),
            image: "agent-dev".to_string(),
            stack: "default".to_string(),
            agent_identifier: "test-agent".to_string(),
            env_json: r#"{"KEY":"VALUE"}"#.to_string(),
            metadata_json: "{}".to_string(),
            ssh_enabled: false,
            ssh_public_key: String::new(),
            web_terminal_enabled: true,
            max_lifetime_seconds: 7200,
            idle_timeout_seconds: 1800,
            cpu_cores: 4,
            memory_mb: 8192,
            disk_gb: 40,
            sidecar_token: "tok".to_string(),
            tee_required: true,
            tee_type: 1,
        };

        // abi_encode() produces tuple encoding (with outer offset prefix).
        let encoded = request.abi_encode();
        let decoded = decode_provision_config(&encoded).unwrap();

        assert_eq!(decoded.name, "tuple-sandbox");
        assert_eq!(decoded.cpu_cores, 4);
        assert_eq!(decoded.memory_mb, 8192);
        assert!(decoded.tee_required);
        assert_eq!(decoded.tee_type, 1);
    }

    #[test]
    fn decode_provision_config_malformed_bytes_rejected() {
        let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
        let result = decode_provision_config(&garbage);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Failed to decode"), "got: {err}");
    }
}
