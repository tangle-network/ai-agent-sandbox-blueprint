//! Escrow watchdog: monitors subscription escrow balance via RPC and
//! auto-deprovisions the instance sandbox when escrow is exhausted for too long.
//!
//! The watchdog polls `getServiceEscrow(serviceId)` and the blueprint's
//! `subscriptionRate` on each tick. If `escrow.balance < subscriptionRate`
//! for `max_consecutive_failures` consecutive checks, the watchdog triggers
//! `deprovision_core(None)` to shut down the sandbox gracefully.
//!
//! Writes `billing_status.json` to the state directory on each tick for
//! external observability (monitoring, UI, etc.).
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
    /// Warn when balance covers fewer than this many billing periods.
    /// Set to 0 to disable low-balance warnings. Default: 3.
    pub low_balance_multiplier: u32,
    /// Grace period (seconds) between deprovision decision and actual teardown.
    /// Allows in-flight requests to complete. Default: 30. Set to 0 to disable.
    pub deprovision_grace_period_secs: u64,
}

impl EscrowWatchdogConfig {
    /// Validate configuration. Returns an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.check_interval_secs == 0 {
            return Err("check_interval_secs must be > 0 (would cause busy-loop)".into());
        }
        if self.max_consecutive_failures == 0 {
            return Err("max_consecutive_failures must be > 0 (would never deprovision)".into());
        }
        if self.http_rpc_endpoint.is_empty() {
            return Err("http_rpc_endpoint must not be empty".into());
        }
        Ok(())
    }

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

        let low_balance_multiplier = std::env::var("ESCROW_LOW_BALANCE_MULTIPLIER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let deprovision_grace_period_secs = std::env::var("ESCROW_DEPROVISION_GRACE_PERIOD_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        Some(Self {
            tangle_contract,
            http_rpc_endpoint,
            service_id,
            blueprint_id,
            check_interval_secs,
            max_consecutive_failures,
            low_balance_multiplier,
            deprovision_grace_period_secs,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Escrow status (returned by check_escrow for observability)
// ─────────────────────────────────────────────────────────────────────────────

/// Result of an escrow balance check, with full balance/rate data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscrowStatus {
    pub balance: U256,
    pub rate: U256,
    pub sufficient: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tick result
// ─────────────────────────────────────────────────────────────────────────────

/// Outcome of a single watchdog tick, returned for observability and testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchdogTickResult {
    /// Escrow balance >= subscription rate. Counter was reset.
    Sufficient {
        /// How many consecutive failures were cleared (0 if none).
        previous_failures: u32,
    },
    /// Escrow is sufficient but running low (balance < rate * multiplier).
    LowBalance {
        balance: U256,
        rate: U256,
        /// Approximate billing periods remaining (balance / rate).
        periods_remaining: u64,
        /// How many consecutive failures were cleared (0 if none).
        previous_failures: u32,
    },
    /// Escrow balance < subscription rate, but below deprovision threshold.
    Insufficient {
        /// Current consecutive failure count (after increment).
        consecutive: u32,
        /// Threshold at which deprovision triggers.
        threshold: u32,
    },
    /// Consecutive failures reached the threshold — deprovision should fire.
    DeprovisionRequired {
        /// How many consecutive failures accumulated.
        consecutive: u32,
    },
    /// RPC or transient error. Counter is NOT modified.
    TransientError(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Billing status file (written to state dir for external observability)
// ─────────────────────────────────────────────────────────────────────────────

/// Write billing status to `billing_status.json` in the state directory.
/// Best-effort — failures are logged but don't affect watchdog operation.
fn write_billing_status(result: &WatchdogTickResult, config: &EscrowWatchdogConfig) {
    use serde_json::json;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (status, balance, rate, consecutive_failures, periods_remaining) = match result {
        WatchdogTickResult::Sufficient { .. } => ("sufficient", None, None, 0u32, None),
        WatchdogTickResult::LowBalance {
            balance,
            rate,
            periods_remaining,
            ..
        } => (
            "low_balance",
            Some(format!("{balance}")),
            Some(format!("{rate}")),
            0,
            Some(*periods_remaining),
        ),
        WatchdogTickResult::Insufficient {
            consecutive,
            threshold: _,
        } => ("insufficient", None, None, *consecutive, None),
        WatchdogTickResult::DeprovisionRequired { consecutive } => {
            ("deprovision_required", None, None, *consecutive, None)
        }
        WatchdogTickResult::TransientError(_) => ("rpc_error", None, None, 0, None),
    };

    let value = json!({
        "status": status,
        "service_id": config.service_id,
        "blueprint_id": config.blueprint_id,
        "balance": balance,
        "rate": rate,
        "consecutive_failures": consecutive_failures,
        "max_consecutive_failures": config.max_consecutive_failures,
        "periods_remaining": periods_remaining,
        "updated_at": now,
    });

    let path = sandbox_runtime::store::state_dir().join("billing_status.json");
    if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap_or_default())
    {
        warn!("escrow-watchdog: failed to write billing status: {e}");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EscrowWatchdog (struct-based, testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Instance-scoped escrow watchdog with observable tick results.
pub struct EscrowWatchdog {
    pub config: EscrowWatchdogConfig,
    failure_count: AtomicU32,
}

impl EscrowWatchdog {
    pub fn new(config: EscrowWatchdogConfig) -> Self {
        Self {
            config,
            failure_count: AtomicU32::new(0),
        }
    }

    /// Current consecutive failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::Relaxed)
    }

    /// Reset the failure counter to zero.
    pub fn reset_failure_count(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
    }

    /// Run a single tick: check escrow, update counter, return the result.
    pub async fn tick(&self) -> WatchdogTickResult {
        match check_escrow(&self.config).await {
            Ok(status) => {
                info!(
                    "escrow-watchdog: balance={}, rate={}, sufficient={}",
                    status.balance, status.rate, status.sufficient
                );

                if status.sufficient {
                    let prev = self.failure_count.swap(0, Ordering::Relaxed);
                    if prev > 0 {
                        info!(
                            "escrow-watchdog: escrow balance recovered after {prev} consecutive failures"
                        );
                    }

                    // Check for low-balance warning
                    if self.config.low_balance_multiplier > 0 && status.rate > U256::ZERO {
                        let threshold =
                            status.rate * U256::from(self.config.low_balance_multiplier);
                        if status.balance < threshold {
                            let periods_remaining: u64 = (status.balance / status.rate)
                                .try_into()
                                .unwrap_or(u64::MAX);
                            warn!(
                                "escrow-watchdog: low balance — ~{periods_remaining} billing periods remaining (threshold: {}x rate)",
                                self.config.low_balance_multiplier
                            );
                            return WatchdogTickResult::LowBalance {
                                balance: status.balance,
                                rate: status.rate,
                                periods_remaining,
                                previous_failures: prev,
                            };
                        }
                    }

                    WatchdogTickResult::Sufficient {
                        previous_failures: prev,
                    }
                } else {
                    let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= self.config.max_consecutive_failures {
                        error!(
                            "escrow-watchdog: escrow exhausted for {count} consecutive checks — deprovision required (balance={}, rate={})",
                            status.balance, status.rate
                        );
                        WatchdogTickResult::DeprovisionRequired { consecutive: count }
                    } else {
                        warn!(
                            "escrow-watchdog: escrow insufficient ({count}/{} consecutive failures, balance={}, rate={})",
                            self.config.max_consecutive_failures, status.balance, status.rate
                        );
                        WatchdogTickResult::Insufficient {
                            consecutive: count,
                            threshold: self.config.max_consecutive_failures,
                        }
                    }
                }
            }
            Err(e) => {
                warn!("escrow-watchdog: RPC error (will retry): {e}");
                WatchdogTickResult::TransientError(e)
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Standalone check (used by EscrowWatchdog::tick and directly in tests)
// ─────────────────────────────────────────────────────────────────────────────

/// Check escrow balance against subscription rate.
/// Returns `EscrowStatus` with balance, rate, and whether escrow is sufficient.
pub async fn check_escrow(config: &EscrowWatchdogConfig) -> Result<EscrowStatus, String> {
    use blueprint_sdk::alloy::providers::ProviderBuilder;

    let url: reqwest::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new().connect_http(url);

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

    let sufficient = if rate == U256::ZERO {
        true
    } else {
        balance >= rate
    };

    Ok(EscrowStatus {
        balance,
        rate,
        sufficient,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Production spawner (calls deprovision_core on threshold)
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn the escrow watchdog as a background task.
///
/// Accepts a shutdown receiver — when the sender is dropped or sends `()`,
/// the watchdog exits cleanly. When the consecutive failure threshold is
/// reached, waits for the grace period then triggers `deprovision_core`.
pub fn spawn_watchdog(
    config: EscrowWatchdogConfig,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    let interval = Duration::from_secs(config.check_interval_secs);
    let grace_period = Duration::from_secs(config.deprovision_grace_period_secs);
    let watchdog = EscrowWatchdog::new(config);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        info!(
            "escrow-watchdog: started (check every {}s, deprovision after {} failures, grace period {}s, low-balance warning at {}x rate)",
            watchdog.config.check_interval_secs,
            watchdog.config.max_consecutive_failures,
            watchdog.config.deprovision_grace_period_secs,
            watchdog.config.low_balance_multiplier
        );

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let result = watchdog.tick().await;
                    write_billing_status(&result, &watchdog.config);

                    if let WatchdogTickResult::DeprovisionRequired { .. } = result {
                        trigger_deprovision(grace_period).await;
                        return;
                    }
                }
                _ = shutdown.recv() => {
                    info!("escrow-watchdog: shutdown signal received, exiting");
                    return;
                }
            }
        }
    })
}

/// Trigger graceful deprovision of the instance sandbox.
/// Waits for the grace period to let in-flight requests complete.
async fn trigger_deprovision(grace_period: Duration) {
    if !grace_period.is_zero() {
        warn!(
            "escrow-watchdog: deprovisioning in {}s (grace period for in-flight requests)",
            grace_period.as_secs()
        );
        tokio::time::sleep(grace_period).await;
    }

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
