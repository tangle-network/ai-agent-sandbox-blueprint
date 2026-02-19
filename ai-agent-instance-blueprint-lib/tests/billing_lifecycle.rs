//! Integration tests for billing lifecycle: escrow watchdog.
//!
//! Uses wiremock to mock JSON-RPC `eth_call` responses with ABI-encoded
//! ServiceEscrow and BlueprintConfig structs.
//!
//! Tests the EscrowWatchdog struct's tick() method for all state transitions:
//! sufficient, insufficient, threshold, recovery, RPC errors, low-balance
//! warnings, config validation, and edge cases.

#![cfg(feature = "billing")]

use ai_agent_instance_blueprint_lib::billing::{
    self, EscrowWatchdog, EscrowWatchdogConfig, ITangleRead, WatchdogTickResult,
};
use blueprint_sdk::alloy::primitives::{Address, U256};
use blueprint_sdk::alloy::sol_types::{SolCall, SolValue};
use serde_json::{json, Value};
use wiremock::matchers::method;
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

// ─────────────────────────────────────────────────────────────────────────────
// Custom matcher: match JSON-RPC eth_call by 4-byte function selector
// ─────────────────────────────────────────────────────────────────────────────

struct SelectorMatcher {
    selector_hex: String,
}

impl SelectorMatcher {
    fn new(selector: [u8; 4]) -> Self {
        Self {
            selector_hex: format!("0x{}", hex::encode(selector)),
        }
    }
}

impl Match for SelectorMatcher {
    fn matches(&self, request: &Request) -> bool {
        let body: Value = match serde_json::from_slice(&request.body) {
            Ok(v) => v,
            Err(_) => return false,
        };
        body.get("params")
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|obj| obj.get("data").or_else(|| obj.get("input")))
            .and_then(|d| d.as_str())
            .map(|data| data.starts_with(&self.selector_hex))
            .unwrap_or(false)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn test_config(rpc_url: &str, max_failures: u32) -> EscrowWatchdogConfig {
    EscrowWatchdogConfig {
        tangle_contract: Address::repeat_byte(0xAA),
        http_rpc_endpoint: rpc_url.to_string(),
        service_id: 42,
        blueprint_id: 7,
        check_interval_secs: 1,
        max_consecutive_failures: max_failures,
        low_balance_multiplier: 3,
        deprovision_grace_period_secs: 0, // no delay in tests
    }
}

fn encode_escrow(balance: U256) -> String {
    let escrow = ITangleRead::ServiceEscrow {
        token: Address::repeat_byte(0x01),
        balance,
        totalDeposited: balance,
        totalReleased: U256::ZERO,
    };
    format!("0x{}", hex::encode(escrow.abi_encode_params()))
}

fn encode_config(subscription_rate: U256) -> String {
    let config = ITangleRead::BlueprintConfig {
        membership: 0,
        pricing: 1,
        minOperators: 1,
        maxOperators: 5,
        subscriptionRate: subscription_rate,
        subscriptionInterval: 2_592_000,
        eventRate: U256::ZERO,
    };
    format!("0x{}", hex::encode(config.abi_encode_params()))
}

fn rpc_ok(result_hex: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": result_hex,
        "id": 1
    })
}

fn escrow_selector() -> [u8; 4] {
    ITangleRead::getServiceEscrowCall::SELECTOR
}

fn config_selector() -> [u8; 4] {
    ITangleRead::getBlueprintConfigCall::SELECTOR
}

async fn mount_rpc_mocks(server: &MockServer, escrow_hex: &str, config_hex: &str) {
    Mock::given(method("POST"))
        .and(SelectorMatcher::new(escrow_selector()))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_ok(escrow_hex)))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(SelectorMatcher::new(config_selector()))
        .respond_with(ResponseTemplate::new(200).set_body_json(rpc_ok(config_hex)))
        .mount(server)
        .await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Config validation tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_config_validate_valid() {
    let config = test_config("http://localhost:8545", 3);
    assert!(config.validate().is_ok());
}

#[test]
fn test_config_validate_zero_interval() {
    let mut config = test_config("http://localhost:8545", 3);
    config.check_interval_secs = 0;
    let err = config.validate().unwrap_err();
    assert!(err.contains("check_interval_secs"), "error: {err}");
}

#[test]
fn test_config_validate_zero_max_failures() {
    let mut config = test_config("http://localhost:8545", 3);
    config.max_consecutive_failures = 0;
    let err = config.validate().unwrap_err();
    assert!(err.contains("max_consecutive_failures"), "error: {err}");
}

#[test]
fn test_config_validate_empty_rpc() {
    let mut config = test_config("http://localhost:8545", 3);
    config.http_rpc_endpoint = String::new();
    let err = config.validate().unwrap_err();
    assert!(err.contains("http_rpc_endpoint"), "error: {err}");
}

// ─────────────────────────────────────────────────────────────────────────────
// check_escrow standalone tests (now returns EscrowStatus)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_check_escrow_sufficient() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1_000_000u64);
    let balance = U256::from(5_000_000u64);
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let status = billing::check_escrow(&config).await.unwrap();
    assert!(status.sufficient, "balance >= rate should be sufficient");
    assert_eq!(status.balance, balance);
    assert_eq!(status.rate, rate);
}

#[tokio::test]
async fn test_check_escrow_exhausted() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1_000_000u64);
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let status = billing::check_escrow(&config).await.unwrap();
    assert!(!status.sufficient, "balance=0 < rate should be insufficient");
    assert_eq!(status.balance, U256::ZERO);
    assert_eq!(status.rate, rate);
}

#[tokio::test]
async fn test_check_escrow_free_service() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::ZERO),
        &encode_config(U256::ZERO),
    )
    .await;

    let status = billing::check_escrow(&config).await.unwrap();
    assert!(status.sufficient, "rate=0 (free) should always be sufficient");
    assert_eq!(status.rate, U256::ZERO);
}

#[tokio::test]
async fn test_check_escrow_exact_balance() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1_000_000u64);
    mount_rpc_mocks(&server, &encode_escrow(rate), &encode_config(rate)).await;

    let status = billing::check_escrow(&config).await.unwrap();
    assert!(
        status.sufficient,
        "balance == rate should be sufficient (>= check)"
    );
}

#[tokio::test]
async fn test_check_escrow_one_wei_short() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1_000_000u64);
    let balance = rate - U256::from(1);
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let status = billing::check_escrow(&config).await.unwrap();
    assert!(
        !status.sufficient,
        "balance = rate-1 should be insufficient"
    );
}

#[tokio::test]
async fn test_check_escrow_rpc_unreachable() {
    let config = EscrowWatchdogConfig {
        tangle_contract: Address::repeat_byte(0xAA),
        http_rpc_endpoint: "http://127.0.0.1:1".to_string(),
        service_id: 42,
        blueprint_id: 7,
        check_interval_secs: 1,
        max_consecutive_failures: 3,
        low_balance_multiplier: 3,
        deprovision_grace_period_secs: 0,
    };

    let result = billing::check_escrow(&config).await;
    assert!(result.is_err(), "unreachable RPC should return Err");
    assert!(
        result.unwrap_err().contains("RPC failed"),
        "error should mention RPC failure"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// EscrowWatchdog tick tests: counter logic and state transitions
// ─────────────────────────────────────────────────────────────────────────────

/// Sufficient escrow → Sufficient result, counter stays 0.
#[tokio::test]
async fn test_tick_sufficient_keeps_counter_zero() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(100u64);
    // balance=1000, rate=100 → 10 periods remaining, above low_balance_multiplier(3)
    mount_rpc_mocks(&server, &encode_escrow(U256::from(1000u64)), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::Sufficient {
            previous_failures: 0
        }
    );
    assert_eq!(watchdog.failure_count(), 0);
}

/// Insufficient escrow → Insufficient result, counter increments to 1.
#[tokio::test]
async fn test_tick_insufficient_increments_counter() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1000u64);
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::Insufficient {
            consecutive: 1,
            threshold: 3,
        }
    );
    assert_eq!(watchdog.failure_count(), 1);
}

/// Three consecutive insufficient ticks → DeprovisionRequired on the third.
#[tokio::test]
async fn test_tick_reaches_threshold_deprovision_required() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    let rate = U256::from(1000u64);
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);

    // Tick 1: insufficient (1/3)
    let r1 = watchdog.tick().await;
    assert_eq!(
        r1,
        WatchdogTickResult::Insufficient {
            consecutive: 1,
            threshold: 3,
        }
    );

    // Tick 2: insufficient (2/3)
    let r2 = watchdog.tick().await;
    assert_eq!(
        r2,
        WatchdogTickResult::Insufficient {
            consecutive: 2,
            threshold: 3,
        }
    );

    // Tick 3: insufficient (3/3) → deprovision
    let r3 = watchdog.tick().await;
    assert_eq!(
        r3,
        WatchdogTickResult::DeprovisionRequired { consecutive: 3 }
    );
    assert_eq!(watchdog.failure_count(), 3);
}

/// Threshold = 1: first insufficient tick immediately returns DeprovisionRequired.
#[tokio::test]
async fn test_tick_threshold_one_immediate_deprovision() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 1);

    let rate = U256::from(1000u64);
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::DeprovisionRequired { consecutive: 1 }
    );
}

/// RPC error → TransientError, counter unchanged.
#[tokio::test]
async fn test_tick_rpc_error_does_not_increment() {
    let config = EscrowWatchdogConfig {
        tangle_contract: Address::repeat_byte(0xAA),
        http_rpc_endpoint: "http://127.0.0.1:1".to_string(),
        service_id: 42,
        blueprint_id: 7,
        check_interval_secs: 1,
        max_consecutive_failures: 3,
        low_balance_multiplier: 3,
        deprovision_grace_period_secs: 0,
    };

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert!(matches!(result, WatchdogTickResult::TransientError(_)));
    assert_eq!(watchdog.failure_count(), 0, "RPC error should not increment counter");
}

/// Insufficient → insufficient → RPC error → insufficient: counter should be 3
/// because RPC errors don't reset or increment. The third insufficient tick
/// sees counter=3 which triggers deprovision.
#[tokio::test]
async fn test_tick_rpc_error_between_failures_preserves_counter() {
    let server = MockServer::start().await;
    let rate = U256::from(1000u64);
    let config = test_config(&server.uri(), 5);
    let watchdog = EscrowWatchdog::new(config);

    // Tick 1: insufficient
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 1, .. }));
    assert_eq!(watchdog.failure_count(), 1);

    // Tick 2: insufficient
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 2, .. }));
    assert_eq!(watchdog.failure_count(), 2);

    // Tick 3: RPC error (reset mocks to return 500)
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("rpc down"))
        .mount(&server)
        .await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::TransientError(_)));
    assert_eq!(watchdog.failure_count(), 2, "RPC error must not change counter");

    // Tick 4: insufficient again — counter continues from 2, now 3
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 3, .. }));
    assert_eq!(watchdog.failure_count(), 3);
}

/// Insufficient x2 → sufficient → counter resets. Next insufficient starts from 1.
#[tokio::test]
async fn test_tick_recovery_resets_counter() {
    let server = MockServer::start().await;
    let rate = U256::from(1000u64);
    let config = test_config(&server.uri(), 5);
    let watchdog = EscrowWatchdog::new(config);

    // Tick 1-2: insufficient
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    watchdog.tick().await;
    watchdog.tick().await;
    assert_eq!(watchdog.failure_count(), 2);

    // Tick 3: sufficient — escrow refunded (balance=5000, rate=1000 → 5 periods, above multiplier=3)
    server.reset().await;
    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::from(5000u64)),
        &encode_config(rate),
    )
    .await;
    let result = watchdog.tick().await;
    assert_eq!(
        result,
        WatchdogTickResult::Sufficient {
            previous_failures: 2,
        }
    );
    assert_eq!(watchdog.failure_count(), 0, "recovery should reset counter");

    // Tick 4: insufficient again — starts fresh from 1
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    let result = watchdog.tick().await;
    assert_eq!(
        result,
        WatchdogTickResult::Insufficient {
            consecutive: 1,
            threshold: 5,
        }
    );
}

/// Full sequence: insufficient → threshold → deprovision, then counter continues past threshold.
#[tokio::test]
async fn test_tick_deprovision_fires_at_exact_threshold() {
    let server = MockServer::start().await;
    let rate = U256::from(1000u64);
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let config = test_config(&server.uri(), 2);
    let watchdog = EscrowWatchdog::new(config);

    // Tick 1: insufficient (1/2)
    let r1 = watchdog.tick().await;
    assert!(matches!(r1, WatchdogTickResult::Insufficient { consecutive: 1, threshold: 2 }));

    // Tick 2: deprovision (2/2)
    let r2 = watchdog.tick().await;
    assert_eq!(r2, WatchdogTickResult::DeprovisionRequired { consecutive: 2 });

    // Tick 3: still deprovision (counter keeps incrementing past threshold)
    let r3 = watchdog.tick().await;
    assert_eq!(r3, WatchdogTickResult::DeprovisionRequired { consecutive: 3 });
}

/// Free service: even with zero balance, tick always returns Sufficient.
#[tokio::test]
async fn test_tick_free_service_always_sufficient() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri(), 3);

    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::ZERO),
        &encode_config(U256::ZERO),
    )
    .await;

    let watchdog = EscrowWatchdog::new(config);

    for _ in 0..5 {
        let result = watchdog.tick().await;
        assert!(matches!(result, WatchdogTickResult::Sufficient { .. }));
    }
    assert_eq!(watchdog.failure_count(), 0);
}

/// Mixed scenario: sufficient, insufficient, rpc error, insufficient, sufficient, insufficient x3 → deprovision
#[tokio::test]
async fn test_tick_full_mixed_scenario() {
    let server = MockServer::start().await;
    let rate = U256::from(1000u64);
    let config = test_config(&server.uri(), 3);
    let watchdog = EscrowWatchdog::new(config);

    // 1. Sufficient (balance=5000, rate=1000 → 5 periods, above multiplier=3)
    mount_rpc_mocks(&server, &encode_escrow(U256::from(5000u64)), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Sufficient { previous_failures: 0 }));
    assert_eq!(watchdog.failure_count(), 0);

    // 2. Insufficient (counter=1)
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 1, .. }));

    // 3. RPC error (counter stays 1)
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::TransientError(_)));
    assert_eq!(watchdog.failure_count(), 1);

    // 4. Insufficient (counter=2)
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 2, .. }));

    // 5. Sufficient — recovery (resets counter, balance=5000 → 5 periods, above multiplier)
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::from(5000u64)), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Sufficient { previous_failures: 2 }));
    assert_eq!(watchdog.failure_count(), 0);

    // 6-8. Insufficient x3 → deprovision (fresh counter)
    server.reset().await;
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 1, .. }));
    assert!(matches!(watchdog.tick().await, WatchdogTickResult::Insufficient { consecutive: 2, .. }));
    assert_eq!(watchdog.tick().await, WatchdogTickResult::DeprovisionRequired { consecutive: 3 });
}

// ─────────────────────────────────────────────────────────────────────────────
// Low-balance warning tests
// ─────────────────────────────────────────────────────────────────────────────

/// Balance = 2x rate with multiplier=3 → LowBalance (2 periods < 3 threshold).
#[tokio::test]
async fn test_tick_low_balance_warning() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 3);
    config.low_balance_multiplier = 3;

    let rate = U256::from(1000u64);
    let balance = U256::from(2000u64); // 2 periods remaining < 3 multiplier
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::LowBalance {
            balance,
            rate,
            periods_remaining: 2,
            previous_failures: 0,
        }
    );
    assert_eq!(watchdog.failure_count(), 0, "low balance is still sufficient — counter stays 0");
}

/// Balance = exactly rate with multiplier=3 → LowBalance (1 period remaining).
#[tokio::test]
async fn test_tick_low_balance_exactly_one_period() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 3);
    config.low_balance_multiplier = 3;

    let rate = U256::from(1000u64);
    mount_rpc_mocks(&server, &encode_escrow(rate), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::LowBalance {
            balance: rate,
            rate,
            periods_remaining: 1,
            previous_failures: 0,
        }
    );
}

/// Balance = 3x rate with multiplier=3 → Sufficient (not low, exactly at threshold).
#[tokio::test]
async fn test_tick_balance_at_multiplier_boundary_is_sufficient() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 3);
    config.low_balance_multiplier = 3;

    let rate = U256::from(1000u64);
    let balance = U256::from(3000u64); // exactly 3x rate = multiplier threshold
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    // balance(3000) >= rate*multiplier(3000), so NOT low balance
    assert_eq!(
        result,
        WatchdogTickResult::Sufficient {
            previous_failures: 0
        }
    );
}

/// Low-balance disabled (multiplier=0) → always Sufficient when balance >= rate.
#[tokio::test]
async fn test_tick_low_balance_disabled() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 3);
    config.low_balance_multiplier = 0;

    let rate = U256::from(1000u64);
    let balance = U256::from(1001u64); // just above rate, would trigger low-balance if enabled
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::Sufficient {
            previous_failures: 0
        }
    );
}

/// Low-balance after recovery: previous_failures is preserved in LowBalance result.
#[tokio::test]
async fn test_tick_low_balance_after_recovery() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 5);
    config.low_balance_multiplier = 3;

    let rate = U256::from(1000u64);
    let watchdog = EscrowWatchdog::new(config);

    // Tick 1-2: insufficient
    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;
    watchdog.tick().await;
    watchdog.tick().await;
    assert_eq!(watchdog.failure_count(), 2);

    // Tick 3: recovery but low balance (balance=1500, rate=1000 → 1 period < 3 multiplier)
    server.reset().await;
    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::from(1500u64)),
        &encode_config(rate),
    )
    .await;
    let result = watchdog.tick().await;
    assert_eq!(
        result,
        WatchdogTickResult::LowBalance {
            balance: U256::from(1500u64),
            rate,
            periods_remaining: 1,
            previous_failures: 2,
        }
    );
    assert_eq!(watchdog.failure_count(), 0, "counter resets even on low balance");
}

/// Free service (rate=0) → no low-balance warning regardless of multiplier.
#[tokio::test]
async fn test_tick_free_service_no_low_balance() {
    let server = MockServer::start().await;
    let mut config = test_config(&server.uri(), 3);
    config.low_balance_multiplier = 10;

    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::ZERO),
        &encode_config(U256::ZERO),
    )
    .await;

    let watchdog = EscrowWatchdog::new(config);
    let result = watchdog.tick().await;

    assert_eq!(
        result,
        WatchdogTickResult::Sufficient {
            previous_failures: 0
        }
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Shutdown test
// ─────────────────────────────────────────────────────────────────────────────

/// spawn_watchdog exits cleanly when shutdown signal is sent.
#[tokio::test]
async fn test_spawn_watchdog_shutdown() {
    let server = MockServer::start().await;
    let rate = U256::from(100u64);
    let balance = U256::from(10000u64);
    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let mut config = test_config(&server.uri(), 3);
    config.check_interval_secs = 60; // long interval so we test shutdown, not ticking

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let handle = billing::spawn_watchdog(config, shutdown_rx);

    // Give it a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!handle.is_finished(), "watchdog should be running");

    // Send shutdown
    shutdown_tx.send(()).unwrap();

    // Should exit within a reasonable time
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "watchdog should exit after shutdown signal");
}
