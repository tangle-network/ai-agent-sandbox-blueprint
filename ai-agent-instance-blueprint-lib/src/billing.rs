//! Escrow watchdog: monitors subscription escrow balance via RPC and
//! auto-deprovisions the instance sandbox when escrow is exhausted for too long.
//!
//! The watchdog polls `getServiceEscrow(serviceId)` and the blueprint's
//! `subscriptionRate` on each tick. If `escrow.balance < subscriptionRate`
//! for `max_consecutive_failures` consecutive checks, the watchdog triggers
//! `deprovision_core(None)` to shut down the sandbox gracefully.
//!
//! Gated behind the `billing` feature flag.

use blueprint_sdk::alloy::primitives::{Address, U256};
use blueprint_sdk::alloy::sol;
use blueprint_sdk::{error, info, warn};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// ABI types for read-only RPC calls
// ─────────────────────────────────────────────────────────────────────────────

sol! {
    #[sol(rpc)]
    interface ITangleRead {
        struct ServiceEscrow {
            address token;
            uint256 balance;
            uint256 totalDeposited;
            uint256 totalReleased;
        }

        struct BlueprintConfig {
            uint8 membership;
            uint8 pricing;
            uint32 minOperators;
            uint32 maxOperators;
            uint256 subscriptionRate;
            uint64 subscriptionInterval;
            uint256 eventRate;
        }

        function getServiceEscrow(uint64 serviceId) external view returns (ServiceEscrow memory);
        function getBlueprintConfig(uint64 blueprintId) external view returns (BlueprintConfig memory);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the escrow watchdog keeper.
#[derive(Debug, Clone)]
pub struct EscrowWatchdogConfig {
    /// Tangle core contract address on-chain.
    pub tangle_contract: Address,
    /// HTTP RPC endpoint for read-only calls.
    pub http_rpc_endpoint: String,
    /// Service ID to monitor.
    pub service_id: u64,
    /// Blueprint ID (needed to look up subscriptionRate).
    pub blueprint_id: u64,
    /// How often to check escrow balance (seconds). Default: 300 (5 min).
    pub check_interval_secs: u64,
    /// How many consecutive failures before auto-deprovision. Default: 3.
    pub max_consecutive_failures: u32,
}

impl EscrowWatchdogConfig {
    /// Load configuration from environment variables.
    /// Returns `None` if `TANGLE_CONTRACT_ADDRESS` is not set (billing disabled).
    pub fn from_env(service_id: u64, blueprint_id: u64) -> Option<Self> {
        let contract_str = std::env::var("TANGLE_CONTRACT_ADDRESS").ok()?;
        let tangle_contract: Address = contract_str.parse().ok()?;

        let http_rpc_endpoint = std::env::var("HTTP_RPC_ENDPOINT")
            .or_else(|_| std::env::var("RPC_URL"))
            .unwrap_or_else(|_| "http://127.0.0.1:8545".to_string());

        let check_interval_secs = std::env::var("ESCROW_CHECK_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        let max_consecutive_failures = std::env::var("ESCROW_MAX_CONSECUTIVE_FAILURES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        Some(Self {
            tangle_contract,
            http_rpc_endpoint,
            service_id,
            blueprint_id,
            check_interval_secs,
            max_consecutive_failures,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Watchdog state
// ─────────────────────────────────────────────────────────────────────────────

/// Shared counter for consecutive escrow-insufficient checks.
static FAILURE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Check escrow balance against subscription rate.
/// Returns `Ok(true)` if escrow is sufficient, `Ok(false)` if insufficient.
pub async fn check_escrow(config: &EscrowWatchdogConfig) -> Result<bool, String> {
    use blueprint_sdk::alloy::providers::ProviderBuilder;

    let url: reqwest::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new()
        .connect_http(url);

    let contract = ITangleRead::new(config.tangle_contract, &provider);

    let escrow = contract
        .getServiceEscrow(config.service_id)
        .call()
        .await
        .map_err(|e| format!("getServiceEscrow RPC failed: {e}"))?;

    let bp_config = contract
        .getBlueprintConfig(config.blueprint_id)
        .call()
        .await
        .map_err(|e| format!("getBlueprintConfig RPC failed: {e}"))?;

    let balance = escrow.balance;
    let rate = bp_config.subscriptionRate;

    if rate == U256::ZERO {
        // Free service or misconfigured — nothing to enforce
        return Ok(true);
    }

    Ok(balance >= rate)
}

/// Single tick of the escrow watchdog.
/// Call this periodically from a `tokio::spawn` interval loop.
pub async fn escrow_watchdog_tick(config: &EscrowWatchdogConfig) {
    match check_escrow(config).await {
        Ok(true) => {
            // Escrow is sufficient — reset failure counter
            let prev = FAILURE_COUNT.swap(0, Ordering::Relaxed);
            if prev > 0 {
                info!(
                    "escrow-watchdog: escrow balance recovered after {prev} consecutive failures"
                );
            }
        }
        Ok(false) => {
            let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            warn!(
                "escrow-watchdog: escrow insufficient ({count}/{} consecutive failures)",
                config.max_consecutive_failures
            );

            if count >= config.max_consecutive_failures {
                error!(
                    "escrow-watchdog: escrow exhausted for {} consecutive checks — auto-deprovisioning",
                    count
                );
                trigger_deprovision().await;
            }
        }
        Err(e) => {
            // RPC errors don't count as escrow failures — transient network issues
            // shouldn't trigger deprovision. Just log and retry next tick.
            warn!("escrow-watchdog: RPC error (will retry): {e}");
        }
    }
}

/// Trigger graceful deprovision of the instance sandbox.
async fn trigger_deprovision() {
    info!("escrow-watchdog: triggering auto-deprovision");

    match crate::deprovision_core(None).await {
        Ok(_) => {
            info!("escrow-watchdog: sandbox deprovisioned successfully");
        }
        Err(e) => {
            error!("escrow-watchdog: deprovision failed: {e}");
        }
    }
}

/// Spawn the escrow watchdog as a background task.
/// Returns `None` if billing is not configured (TANGLE_CONTRACT_ADDRESS not set).
pub fn spawn_watchdog(
    config: EscrowWatchdogConfig,
) -> tokio::task::JoinHandle<()> {
    let interval = Duration::from_secs(config.check_interval_secs);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        info!(
            "escrow-watchdog: started (check every {}s, deprovision after {} failures)",
            config.check_interval_secs, config.max_consecutive_failures
        );

        loop {
            ticker.tick().await;
            escrow_watchdog_tick(&config).await;
        }
    })
}
