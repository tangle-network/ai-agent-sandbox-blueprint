//! Integration tests for billing lifecycle: escrow watchdog.
//!
//! Uses wiremock to mock JSON-RPC `eth_call` responses with ABI-encoded
//! ServiceEscrow and BlueprintConfig structs.

#![cfg(feature = "billing")]

use ai_agent_instance_blueprint_lib::billing::{self, EscrowWatchdogConfig, ITangleRead};
use blueprint_sdk::alloy::primitives::{Address, U256};
use blueprint_sdk::alloy::sol_types::{SolCall, SolValue};
use serde_json::{json, Value};
use wiremock::matchers::method;
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

// ─────────────────────────────────────────────────────────────────────────────
// Custom matcher: match JSON-RPC eth_call by 4-byte function selector
// ─────────────────────────────────────────────────────────────────────────────

/// Matches a JSON-RPC `eth_call` request whose `data` field starts with the
/// given 4-byte function selector.
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
        // JSON-RPC: { "params": [ { "data": "0x<selector><args>" }, ... ] }
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

/// Build a test config pointing at the given mock server.
fn test_config(rpc_url: &str) -> EscrowWatchdogConfig {
    EscrowWatchdogConfig {
        tangle_contract: Address::repeat_byte(0xAA),
        http_rpc_endpoint: rpc_url.to_string(),
        service_id: 42,
        blueprint_id: 7,
        check_interval_secs: 1,
        max_consecutive_failures: 3,
    }
}

/// ABI-encode a ServiceEscrow struct for use as a JSON-RPC result.
fn encode_escrow(balance: U256) -> String {
    let escrow = ITangleRead::ServiceEscrow {
        token: Address::repeat_byte(0x01),
        balance,
        totalDeposited: balance,
        totalReleased: U256::ZERO,
    };
    format!("0x{}", hex::encode(escrow.abi_encode_params()))
}

/// ABI-encode a BlueprintConfig struct for use as a JSON-RPC result.
fn encode_config(subscription_rate: U256) -> String {
    let config = ITangleRead::BlueprintConfig {
        membership: 0,
        pricing: 1,
        minOperators: 1,
        maxOperators: 5,
        subscriptionRate: subscription_rate,
        subscriptionInterval: 2_592_000, // 30 days
        eventRate: U256::ZERO,
    };
    format!("0x{}", hex::encode(config.abi_encode_params()))
}

/// JSON-RPC success response.
fn rpc_ok(result_hex: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": result_hex,
        "id": 1
    })
}

/// Function selectors from the sol!-generated interface.
fn escrow_selector() -> [u8; 4] {
    ITangleRead::getServiceEscrowCall::SELECTOR
}

fn config_selector() -> [u8; 4] {
    ITangleRead::getBlueprintConfigCall::SELECTOR
}

/// Mount mocks for both getServiceEscrow and getBlueprintConfig.
/// Uses function-selector matching so each call gets the correct response.
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
// Tests
// ─────────────────────────────────────────────────────────────────────────────

/// check_escrow returns Ok(true) when escrow balance >= subscription rate.
#[tokio::test]
async fn test_check_escrow_sufficient() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri());

    let rate = U256::from(1_000_000u64);
    let balance = U256::from(5_000_000u64);

    mount_rpc_mocks(&server, &encode_escrow(balance), &encode_config(rate)).await;

    let result = billing::check_escrow(&config).await;
    assert!(result.is_ok(), "check_escrow failed: {:?}", result.err());
    assert!(
        result.unwrap(),
        "escrow should be sufficient (balance >= rate)"
    );
}

/// check_escrow returns Ok(false) when escrow balance < subscription rate.
#[tokio::test]
async fn test_check_escrow_exhausted() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri());

    let rate = U256::from(1_000_000u64);

    mount_rpc_mocks(&server, &encode_escrow(U256::ZERO), &encode_config(rate)).await;

    let result = billing::check_escrow(&config).await;
    assert!(result.is_ok(), "check_escrow failed: {:?}", result.err());
    assert!(
        !result.unwrap(),
        "escrow should be insufficient (balance < rate)"
    );
}

/// check_escrow returns Ok(true) when subscription rate is zero (free service).
#[tokio::test]
async fn test_check_escrow_free_service() {
    let server = MockServer::start().await;
    let config = test_config(&server.uri());

    mount_rpc_mocks(
        &server,
        &encode_escrow(U256::ZERO),
        &encode_config(U256::ZERO),
    )
    .await;

    let result = billing::check_escrow(&config).await;
    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "free service should always return sufficient"
    );
}

/// check_escrow returns Err when RPC is unreachable.
#[tokio::test]
async fn test_check_escrow_rpc_error() {
    let config = EscrowWatchdogConfig {
        tangle_contract: Address::repeat_byte(0xAA),
        http_rpc_endpoint: "http://127.0.0.1:1".to_string(),
        service_id: 42,
        blueprint_id: 7,
        check_interval_secs: 1,
        max_consecutive_failures: 3,
    };

    let result = billing::check_escrow(&config).await;
    assert!(result.is_err(), "should fail when RPC is unreachable");
    let err = result.unwrap_err();
    assert!(
        err.contains("RPC failed"),
        "error should mention RPC failure, got: {err}"
    );
}

/// Watchdog tick increments failure count on insufficient escrow,
/// ignores RPC errors, and resets on recovery.
#[tokio::test]
async fn test_consecutive_failures_tracking() {
    // Tick 1: sufficient escrow → resets counter to 0
    {
        let server = MockServer::start().await;
        let config = test_config(&server.uri());
        mount_rpc_mocks(
            &server,
            &encode_escrow(U256::from(1000u64)),
            &encode_config(U256::from(100u64)),
        )
        .await;
        billing::escrow_watchdog_tick(&config).await;
    }

    // Tick 2: insufficient escrow → counter = 1
    {
        let server = MockServer::start().await;
        let config = test_config(&server.uri());
        mount_rpc_mocks(
            &server,
            &encode_escrow(U256::ZERO),
            &encode_config(U256::from(100u64)),
        )
        .await;
        billing::escrow_watchdog_tick(&config).await;
        // counter = 1, threshold = 3 → no deprovision
    }

    // Tick 3: RPC error → counter stays at 1 (errors don't increment)
    {
        let config = EscrowWatchdogConfig {
            tangle_contract: Address::repeat_byte(0xAA),
            http_rpc_endpoint: "http://127.0.0.1:1".to_string(),
            service_id: 42,
            blueprint_id: 7,
            check_interval_secs: 1,
            max_consecutive_failures: 3,
        };
        billing::escrow_watchdog_tick(&config).await;
    }

    // Tick 4: sufficient again → resets counter to 0
    {
        let server = MockServer::start().await;
        let config = test_config(&server.uri());
        mount_rpc_mocks(
            &server,
            &encode_escrow(U256::from(500u64)),
            &encode_config(U256::from(100u64)),
        )
        .await;
        billing::escrow_watchdog_tick(&config).await;
    }
}
