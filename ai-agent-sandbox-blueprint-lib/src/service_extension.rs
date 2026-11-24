//! On-chain service lifetime tracking (issue #239).
//!
//! Cloud sandboxes are prepaid on-chain: a service's paid window is
//! `createdAt + ttl` on the Tangle core contract. This module keeps local
//! sandbox lifetimes aligned with that window:
//!
//! - [`clamp_lifetime_to_paid_time`] caps a creation request's
//!   `max_lifetime_seconds` to the service's remaining paid time and fails
//!   the creation when nothing is left.
//! - [`ServiceExtensionWatcher`] periodically re-reads the window and raises
//!   the `max_lifetime_seconds` of running sandboxes belonging to the
//!   configured service after each on-chain payment that extends the TTL.
//!   Extensions are extend-only: a shrinking window never shortens a local
//!   lifetime, and once the window passes the reaper deletes the sandbox
//!   through its existing `max_lifetime_seconds` check.
//!
//! The subsystem is inactive unless `TANGLE_CONTRACT_ADDRESS` is set.
//! Endpoint resolution mirrors the billing watchdog: `HTTP_RPC_ENDPOINT`,
//! then `RPC_URL`, then `http://127.0.0.1:8545`.

use std::str::FromStr;
use std::time::Duration;

use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol;
use blueprint_sdk::{info, warn};

use crate::runtime::{SandboxState, sandboxes};
use crate::util::now_ts;

/// Tangle core contract address; unset disables service lifetime tracking.
const ENV_CONTRACT_ADDRESS: &str = "TANGLE_CONTRACT_ADDRESS";
/// Preferred HTTP RPC endpoint for on-chain reads.
const ENV_HTTP_RPC_ENDPOINT: &str = "HTTP_RPC_ENDPOINT";
/// Fallback HTTP RPC endpoint for on-chain reads.
const ENV_RPC_URL: &str = "RPC_URL";
/// Seconds between extension watcher ticks (default 300).
const ENV_INTERVAL_SECS: &str = "SERVICE_EXTENSION_INTERVAL_SECS";

const DEFAULT_HTTP_RPC_ENDPOINT: &str = "http://127.0.0.1:8545";
const DEFAULT_INTERVAL_SECS: u64 = 300;

// Minimal `Types.Service` view: only the payment-window fields are read.
// Enum members are declared `uint8`, exactly how Solidity encodes them in
// the ABI, so no enum codegen is needed.
sol! {
    #[sol(rpc)]
    interface ITangleServiceReader {
        struct Service {
            uint64 blueprintId;
            address owner;
            uint64 createdAt;
            uint64 ttl;
            uint64 terminatedAt;
            uint64 lastPaymentAt;
            uint32 operatorCount;
            uint32 minOperators;
            uint32 maxOperators;
            uint8 membership;
            uint8 pricing;
            uint8 status;
            uint8 confidentiality;
        }

        function getService(uint64 serviceId) external view returns (Service memory service);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the service extension watcher.
#[derive(Clone, Debug)]
pub struct ServiceExtensionConfig {
    /// On-chain service whose paid window drives local lifetimes.
    pub service_id: u64,
    /// Tangle core contract holding the `_services` mapping.
    pub contract_address: Address,
    /// HTTP RPC endpoint used for all on-chain reads.
    pub endpoint: String,
    /// Delay between watcher ticks.
    pub interval: Duration,
}

impl ServiceExtensionConfig {
    /// Build the config from the environment for `service_id`.
    ///
    /// Returns `None` when [`ENV_CONTRACT_ADDRESS`] is unset or unparsable,
    /// which disables the whole subsystem (no clamping, no watcher).
    pub fn from_env(service_id: u64) -> Option<Self> {
        let contract_address =
            Address::from_str(&std::env::var(ENV_CONTRACT_ADDRESS).ok()?).ok()?;
        let endpoint = std::env::var(ENV_HTTP_RPC_ENDPOINT)
            .or_else(|_| std::env::var(ENV_RPC_URL))
            .unwrap_or_else(|_| DEFAULT_HTTP_RPC_ENDPOINT.to_string());
        let interval = Duration::from_secs(
            std::env::var(ENV_INTERVAL_SECS)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_INTERVAL_SECS),
        );
        Some(Self {
            service_id,
            contract_address,
            endpoint,
            interval,
        })
    }

    /// Reject configs that could never make progress.
    pub fn validate(&self) -> Result<(), String> {
        if self.interval.is_zero() {
            return Err(format!("{ENV_INTERVAL_SECS} must be greater than zero"));
        }
        if self.endpoint.trim().is_empty() {
            return Err(format!("{ENV_HTTP_RPC_ENDPOINT} must not be empty"));
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// On-chain reads and pure helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Read the on-chain `Service` for `config.service_id`.
///
/// A fresh provider per call keeps this correct across endpoint restarts,
/// mirroring the billing watchdog's `check_escrow`.
async fn fetch_service(config: &ServiceExtensionConfig) -> Result<Service, String> {
    let provider = ProviderBuilder::new()
        .connect(config.endpoint.as_str())
        .await
        .map_err(|e| format!("failed to connect to {}: {e}", config.endpoint))?;
    ITangleServiceReader::new(config.contract_address, &provider)
        .getService(config.service_id)
        .call()
        .await
        .map_err(|e| format!("getService({}) failed: {e}", config.service_id))
}

/// Absolute end of the service's paid window, or `None` when `ttl == 0`
/// (the service never expires).
fn paid_until(service: &Service) -> Option<u64> {
    (service.ttl != 0).then(|| service.createdAt.saturating_add(service.ttl))
}

/// Remaining paid seconds at `now`; `Some(0)` when the window has passed.
fn remaining_paid_seconds(service: &Service, now: u64) -> Option<u64> {
    paid_until(service).map(|end| end.saturating_sub(now))
}

/// Extend-only lifetime for a sandbox created at `created_at` given the
/// service's paid window: never below the current lifetime.
fn extended_lifetime(created_at: u64, current: u64, paid_until: u64) -> u64 {
    let target = paid_until.saturating_sub(created_at);
    target.max(current)
}

/// Cap a creation-requested lifetime to the service's remaining paid time.
///
/// Disabled (no [`ENV_CONTRACT_ADDRESS`]) or unbounded (`ttl == 0`) services
/// keep the requested lifetime. When the chain is unreachable the clamp
/// fails open — the requested lifetime stands and the watcher reconciles
/// later. A service whose paid window has fully elapsed fails creation.
pub(crate) async fn clamp_lifetime_to_paid_time(
    service_id: u64,
    requested: u64,
) -> Result<u64, String> {
    let Some(config) = ServiceExtensionConfig::from_env(service_id) else {
        return Ok(requested); // subsystem disabled
    };
    config.validate()?;
    let service = match fetch_service(&config).await {
        Ok(service) => service,
        Err(e) => {
            warn!("sandbox_create: lifetime clamp skipped ({e})");
            return Ok(requested); // fail open, watcher reconciles
        }
    };
    match remaining_paid_seconds(&service, now_ts()) {
        None => Ok(requested),
        Some(remaining) if remaining > 0 => Ok(requested.min(remaining)),
        Some(_) => Err(format!(
            "service {service_id} paid window has elapsed (createdAt {}, ttl {}); \
             extend the service before creating sandboxes",
            service.createdAt, service.ttl,
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Watcher
// ─────────────────────────────────────────────────────────────────────────────

/// Observable result of one extension pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickSummary {
    /// Running sandboxes matched to the configured service.
    pub running: usize,
    /// Sandboxes whose lifetime was raised this pass.
    pub extended: usize,
}

/// Service-scoped extension watcher with observable tick results.
pub struct ServiceExtensionWatcher {
    config: ServiceExtensionConfig,
}

impl ServiceExtensionWatcher {
    pub fn new(config: ServiceExtensionConfig) -> Self {
        Self { config }
    }

    /// Run one extension pass: fetch the paid window, then raise the
    /// `max_lifetime_seconds` of every running sandbox bound to the
    /// configured service so its end matches the window.
    pub async fn tick(&self) -> Result<TickSummary, String> {
        let service = fetch_service(&self.config).await?;
        if service.terminatedAt != 0 {
            return Ok(TickSummary::default()); // terminated: no live window
        }
        let Some(paid_until) = paid_until(&service) else {
            return Ok(TickSummary::default()); // ttl == 0: no expiry to track
        };
        let store = sandboxes().map_err(|e| format!("failed to open sandbox store: {e}"))?;
        let mut summary = TickSummary::default();
        let records = store
            .values()
            .map_err(|e| format!("failed to read sandbox store: {e}"))?;
        for record in records {
            if record.state != SandboxState::Running
                || record.service_id != Some(self.config.service_id)
            {
                continue;
            }
            summary.running += 1;
            let target = extended_lifetime(
                record.created_at,
                record.max_lifetime_seconds,
                paid_until,
            );
            if target <= record.max_lifetime_seconds {
                continue;
            }
            let updated = store
                .update(&record.id, |rec| rec.max_lifetime_seconds = target)
                .map_err(|e| format!("failed to extend sandbox {}: {e}", record.id))?;
            if updated {
                summary.extended += 1;
                info!(
                    "service-extension: sandbox {} lifetime raised to {target}s \
                     (paid until {paid_until})",
                    record.id
                );
            }
        }
        Ok(summary)
    }
}

/// Spawn the background extension loop for this operator's service.
///
/// The first tick runs immediately, then every `config.interval`. Ticks that
/// fail only log a warning and retry on the next tick. The task exits when
/// `shutdown` fires. Callers are expected to run [`ServiceExtensionConfig::validate`]
/// first and log invalid configs instead of spawning.
pub fn spawn_extension_watcher(
    config: ServiceExtensionConfig,
    mut shutdown: tokio::sync::watch::Receiver<()>,
) {
    let service_id = config.service_id;
    let interval = config.interval;
    let watcher = ServiceExtensionWatcher::new(config);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        info!(
            "service-extension: watcher started for service {service_id} (tick {}s)",
            interval.as_secs()
        );
        loop {
            tokio::select! {
                _ = ticker.tick() => match watcher.tick().await {
                    Ok(summary) if summary.extended > 0 => {
                        info!(
                            "service-extension: raised {} sandbox lifetime(s)",
                            summary.extended
                        );
                    }
                    Ok(_) => {}
                    Err(e) => warn!("service-extension: tick failed: {e}"),
                },
                _ = shutdown.changed() => {
                    info!("service-extension: shutting down");
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(created_at: u64, ttl: u64) -> Service {
        Service {
            blueprintId: 0,
            owner: Address::ZERO,
            createdAt,
            ttl,
            terminatedAt: 0,
            lastPaymentAt: 0,
            operatorCount: 0,
            minOperators: 0,
            maxOperators: 0,
            membership: 0,
            pricing: 0,
            status: 0,
            confidentiality: 0,
        }
    }

    #[test]
    fn paid_until_tracks_created_plus_ttl() {
        assert_eq!(paid_until(&service(100, 50)), Some(150));
    }

    #[test]
    fn paid_until_none_when_service_never_expires() {
        assert_eq!(paid_until(&service(100, 0)), None);
    }

    #[test]
    fn paid_until_saturates_on_overflow() {
        assert_eq!(paid_until(&service(u64::MAX, 1)), Some(u64::MAX));
    }

    #[test]
    fn remaining_paid_seconds_before_and_after_window() {
        let svc = service(100, 50);
        assert_eq!(remaining_paid_seconds(&svc, 120), Some(30));
        assert_eq!(remaining_paid_seconds(&svc, 150), Some(0));
        assert_eq!(remaining_paid_seconds(&svc, 999), Some(0));
        assert_eq!(remaining_paid_seconds(&service(100, 0), 120), None);
    }

    #[test]
    fn extended_lifetime_raises_to_paid_window() {
        assert_eq!(extended_lifetime(100, 60, 200), 100);
    }

    #[test]
    fn extended_lifetime_never_shrinks() {
        assert_eq!(extended_lifetime(100, 300, 150), 300);
    }

    #[test]
    fn extended_lifetime_saturates_when_created_after_window() {
        assert_eq!(extended_lifetime(500, 60, 200), 60);
    }
}
