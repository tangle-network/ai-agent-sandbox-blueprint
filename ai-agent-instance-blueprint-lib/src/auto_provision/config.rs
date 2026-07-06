use super::*;

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
