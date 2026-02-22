//! Auto-provision: reads service config from BSM on-chain, provisions sandbox
//! automatically on startup without waiting for a manual `JOB_PROVISION` call.
//!
//! Flow:
//! 1. On startup, check if already provisioned (`get_instance_sandbox()`)
//! 2. If not, poll `getServiceConfig(serviceId)` from BSM via RPC
//! 3. When config available, decode as `ProvisionRequest`, call `provision_core()`
//! 4. Store sandbox record via `set_instance_sandbox()`
//!
//! The on-chain `JOB_PROVISION` result is still submitted by the `instance_provision`
//! handler when Tangle delivers the job. The handler is idempotent: if auto-provision
//! already created the sandbox, it returns the existing info.

use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol_types::SolValue;
use blueprint_sdk::{info, warn};
use std::time::Duration;

use crate::tee::TeeBackend;
use crate::{IBsmRead, ProvisionRequest, get_instance_sandbox, provision_core, set_instance_sandbox};

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
/// Use `abi_decode_params` rather than `abi_decode` to match this encoding.
pub fn decode_provision_config(config_bytes: &[u8]) -> Result<ProvisionRequest, String> {
    ProvisionRequest::abi_decode_params(config_bytes)
        .map_err(|e| format!("Failed to decode ProvisionRequest from service config: {e}"))
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
) -> Result<(), String> {
    // Already provisioned?
    if get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        info!("Auto-provision: instance already provisioned, skipping");
        return Ok(());
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
                info!("Auto-provision: service config found ({} bytes)", bytes.len());
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
                warn!("Auto-provision: RPC error (attempt {}/{}): {e}", attempts, config.max_attempts);
                if attempts >= config.max_attempts {
                    return Err(format!("Auto-provision: RPC failed after {} attempts: {e}", config.max_attempts));
                }
            }
        }

        // Check if provisioned by another path (e.g., manual JOB_PROVISION)
        if get_instance_sandbox()
            .map_err(|e| e.to_string())?
            .is_some()
        {
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
    let owner = match read_service_owner(&config).await {
        Ok(addr) if !addr.is_empty() => {
            info!("Auto-provision: service owner = {addr}");
            addr
        }
        Ok(_) => {
            warn!("Auto-provision: no service owner set on-chain, sandbox will be unowned");
            String::new()
        }
        Err(e) => {
            warn!("Auto-provision: failed to read service owner ({e}), sandbox will be unowned");
            String::new()
        }
    };

    // Final check before provisioning (race with manual JOB_PROVISION)
    if get_instance_sandbox()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        info!("Auto-provision: instance was provisioned externally, skipping");
        return Ok(());
    }

    // Provision
    let (output, record) = provision_core(&request, tee, &owner).await?;

    // Store record
    set_instance_sandbox(record).map_err(|e| e.to_string())?;

    info!(
        "Auto-provision: sandbox '{}' created at {} (ssh_port={})",
        output.sandbox_id, output.sidecar_url, output.ssh_port
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_returns_none_without_bsm() {
        // BSM_ADDRESS not set → None
        unsafe { std::env::remove_var("BSM_ADDRESS") };
        assert!(AutoProvisionConfig::from_env(1).is_none());
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
}
