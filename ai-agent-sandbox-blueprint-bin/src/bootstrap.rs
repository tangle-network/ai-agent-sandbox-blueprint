//! Service bootstrap helpers: env/heartbeat config + chain-vs-host capacity validation.

// Only the qos-gated helpers below reference parent-scope items; the capacity
// check is self-contained.
#[cfg(feature = "qos")]
use super::*;

/// Parse a u64 from the first env var that's set in `keys`. Logs a warning
/// and returns `None` if a value is set but doesn't parse — so operators
/// see misconfiguration in observability instead of features silently
/// disabling.
#[cfg(feature = "qos")]
pub(crate) fn parse_required_u64_env(keys: &[&str]) -> Option<u64> {
    for key in keys {
        match std::env::var(key) {
            Ok(raw) => match raw.parse::<u64>() {
                Ok(v) => return Some(v),
                Err(e) => {
                    warn!(
                        env = key,
                        value = %raw,
                        err = %e,
                        "env var is set but not a valid u64; falling back to next key"
                    );
                }
            },
            Err(_) => continue,
        }
    }
    None
}

/// Build heartbeat config from environment variables.
///
/// Required env vars:
///   - `SERVICE_ID` or `TANGLE_SERVICE_ID` — the service instance ID
///   - `BLUEPRINT_ID` or `TANGLE_BLUEPRINT_ID` — the blueprint ID
///   - `STATUS_REGISTRY_ADDRESS` — the OperatorStatusRegistry contract address
///
/// Optional:
///   - `HEARTBEAT_INTERVAL_SECS` — heartbeat interval (default: 120)
///   - `HEARTBEAT_MAX_MISSED` — max missed beats before slashing (default: 3)
#[cfg(feature = "qos")]
pub(crate) fn build_heartbeat_config() -> Option<HeartbeatConfig> {
    use std::str::FromStr;

    let service_id: u64 = parse_required_u64_env(&["SERVICE_ID", "TANGLE_SERVICE_ID"])?;
    let blueprint_id: u64 = parse_required_u64_env(&["BLUEPRINT_ID", "TANGLE_BLUEPRINT_ID"])?;

    let registry_addr_str = std::env::var("STATUS_REGISTRY_ADDRESS").ok()?;
    let status_registry_address =
        match blueprint_sdk::alloy::primitives::Address::from_str(&registry_addr_str) {
            Ok(addr) => addr,
            Err(e) => {
                warn!(
                    value = %registry_addr_str,
                    err = %e,
                    "STATUS_REGISTRY_ADDRESS is set but not a valid EVM address; heartbeat disabled"
                );
                return None;
            }
        };

    let interval_secs: u64 = std::env::var("HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    let max_missed: u32 = std::env::var("HEARTBEAT_MAX_MISSED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    Some(HeartbeatConfig {
        interval_secs,
        jitter_percent: 10,
        service_id,
        blueprint_id,
        max_missed_heartbeats: max_missed,
        status_registry_address,
    })
}

/// Cross-check on-chain capacity vs the host admission cap.
///
/// `OPERATOR_MAX_CAPACITY` is what this operator registers on-chain (the
/// chain may assign that many sandboxes); `SANDBOX_MAX_COUNT` is what the
/// host runtime will actually admit. Registering more than the host admits
/// guarantees rejected work, so startup fails when both are set and
/// chain > host. `SANDBOX_MAX_COUNT=0` (uncapped host) always passes.
/// Unparseable values are ignored here for parity with their consumers:
/// registration skips an unparseable `OPERATOR_MAX_CAPACITY` and the runtime
/// substitutes its default for an unparseable `SANDBOX_MAX_COUNT`.
pub(crate) fn validate_chain_vs_host_capacity(
    operator_max_capacity: Option<&str>,
    sandbox_max_count: Option<&str>,
) -> Result<(), String> {
    let (Some(capacity_raw), Some(max_count_raw)) = (operator_max_capacity, sandbox_max_count)
    else {
        return Ok(());
    };
    let (Ok(capacity), Ok(max_count)) = (
        capacity_raw.trim().parse::<u32>(),
        max_count_raw.trim().parse::<usize>(),
    ) else {
        return Ok(());
    };
    if max_count == 0 || capacity as usize <= max_count {
        return Ok(());
    }
    Err(format!(
        "OPERATOR_MAX_CAPACITY={capacity} exceeds SANDBOX_MAX_COUNT={max_count}: the chain \
         would assign up to {capacity} sandboxes but this host rejects creations beyond \
         {max_count}. Lower OPERATOR_MAX_CAPACITY or raise SANDBOX_MAX_COUNT."
    ))
}
