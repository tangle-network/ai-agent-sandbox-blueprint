//! Anvil integration tests for billing lifecycle.
//!
//! Spins up a real Anvil instance with the full tnt-core stack deployed,
//! then tests the escrow watchdog against actual on-chain state.
//! No mocked JSON-RPC — all calls hit the deployed Tangle contract.
//!
//! Requires Docker (testcontainers) and the tnt-core broadcast artifacts.
//! Gated behind `BILLING_ANVIL=1` env var (slow, needs Docker).

#![cfg(feature = "billing")]

use ai_agent_instance_blueprint_lib::billing::{self, EscrowWatchdogConfig, ITangleRead};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{
    TangleHarness, missing_tnt_core_artifacts, LOCAL_BLUEPRINT_ID, LOCAL_SERVICE_ID,
};
use blueprint_sdk::alloy::primitives::{Address, Bytes, U256};
use blueprint_sdk::alloy::providers::{Provider, ProviderBuilder};
use blueprint_sdk::alloy::sol;
use blueprint_sdk::alloy::sol_types::SolCall;
use once_cell::sync::Lazy;
use serde_json::json;
use std::borrow::Cow;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

// Serialize Anvil tests — only one can run at a time.
static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

// ─────────────────────────────────────────────────────────────────────────────
// ABI for write operations (fundService)
// ─────────────────────────────────────────────────────────────────────────────

sol! {
    #[sol(rpc)]
    interface ITangleWrite {
        function fundService(uint64 serviceId, uint256 amount) external payable;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn should_run() -> bool {
    std::env::var("BILLING_ANVIL").ok().as_deref() == Some("1")
}

async fn spawn_harness() -> Result<Option<TangleHarness>> {
    match TangleHarness::builder()
        .include_anvil_logs(false)
        .seed_from_broadcast(true)
        .spawn()
        .await
    {
        Ok(h) => Ok(Some(h)),
        Err(e) if missing_tnt_core_artifacts(&e) => {
            eprintln!("Skipping billing Anvil test: {e}");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn watchdog_config(rpc_url: &str, tangle_contract: Address) -> EscrowWatchdogConfig {
    EscrowWatchdogConfig {
        tangle_contract,
        http_rpc_endpoint: rpc_url.to_string(),
        service_id: LOCAL_SERVICE_ID,
        blueprint_id: LOCAL_BLUEPRINT_ID,
        check_interval_secs: 1,
        max_consecutive_failures: 3,
    }
}

/// Send a raw JSON-RPC request via the provider.
async fn anvil_rpc<P: Provider>(
    provider: &P,
    method: &'static str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    provider
        .raw_request::<_, serde_json::Value>(Cow::Borrowed(method), params)
        .await
        .context(method)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Verify check_escrow can talk to the real deployed Tangle contract.
/// The seeded service has blueprint_id=0 / service_id=0 with all contracts
/// deployed via the tnt-core LocalTestnet script.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_check_escrow_real_rpc() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set BILLING_ANVIL=1 to run");
        return Ok(());
    }

    let _guard = HARNESS_LOCK.lock().await;

    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let rpc_url = harness.http_endpoint().as_str();
    let config = watchdog_config(rpc_url, harness.tangle_contract);

    // Call check_escrow against the real contract — should not error.
    let result = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // The seeded blueprint may have subscriptionRate=0 (free) — either way,
    // we proved the ABI encoding/decoding works end-to-end.
    eprintln!("check_escrow result: sufficient={result}");

    Ok(())
}

/// Read escrow and config directly via alloy provider, then verify
/// check_escrow agrees with the raw on-chain data.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_check_escrow_matches_raw_reads() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set BILLING_ANVIL=1 to run");
        return Ok(());
    }

    let _guard = HARNESS_LOCK.lock().await;

    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let rpc_url = harness.http_endpoint().as_str();
    let url: reqwest::Url = rpc_url.parse()?;
    let provider = ProviderBuilder::new().connect_http(url);
    let contract = ITangleRead::new(harness.tangle_contract, &provider);

    // Read raw values from chain.
    let escrow = contract
        .getServiceEscrow(LOCAL_SERVICE_ID)
        .call()
        .await
        .context("getServiceEscrow")?;
    let bp_config = contract
        .getBlueprintConfig(LOCAL_BLUEPRINT_ID)
        .call()
        .await
        .context("getBlueprintConfig")?;

    eprintln!(
        "On-chain escrow: balance={}, totalDeposited={}, totalReleased={}",
        escrow.balance, escrow.totalDeposited, escrow.totalReleased
    );
    eprintln!(
        "On-chain config: subscriptionRate={}, subscriptionInterval={}, pricing={}",
        bp_config.subscriptionRate, bp_config.subscriptionInterval, bp_config.pricing
    );

    // Now call check_escrow and verify it agrees.
    let config = watchdog_config(rpc_url, harness.tangle_contract);
    let result = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let expected = if bp_config.subscriptionRate == U256::ZERO {
        true // free service
    } else {
        escrow.balance >= bp_config.subscriptionRate
    };

    assert_eq!(
        result, expected,
        "check_escrow should match raw on-chain comparison"
    );

    Ok(())
}

/// Fund escrow via fundService(), then verify check_escrow returns true.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_check_escrow_after_funding() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set BILLING_ANVIL=1 to run");
        return Ok(());
    }

    let _guard = HARNESS_LOCK.lock().await;

    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let rpc_url = harness.http_endpoint().as_str();
    let url: reqwest::Url = rpc_url.parse()?;

    // Read current subscription rate.
    let provider = ProviderBuilder::new().connect_http(url.clone());
    let read_contract = ITangleRead::new(harness.tangle_contract, &provider);
    let bp_config = read_contract
        .getBlueprintConfig(LOCAL_BLUEPRINT_ID)
        .call()
        .await
        .context("getBlueprintConfig")?;

    let rate = bp_config.subscriptionRate;
    eprintln!("subscriptionRate = {rate}");

    if rate == U256::ZERO {
        // Free service — funding is a no-op. check_escrow should still return true.
        let config = watchdog_config(rpc_url, harness.tangle_contract);
        let result = billing::check_escrow(&config)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        assert!(result, "free service should always be sufficient");
        eprintln!("Free service — funding test skipped (rate=0)");
        return Ok(());
    }

    // Fund escrow with 10x the subscription rate using the service owner account.
    // Anvil account 0 (service owner): 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    let service_owner: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".parse()?;
    let fund_amount = rate * U256::from(10);

    // Use WS provider for Anvil-specific JSON-RPC (impersonation, mining).
    let ws_provider = ProviderBuilder::new()
        .connect(harness.ws_endpoint().as_str())
        .await
        .context("ws connect")?;

    // Impersonate the service owner so we can send a tx without a private key.
    anvil_rpc(
        &ws_provider,
        "anvil_impersonateAccount",
        json!([format!("{:#x}", service_owner)]),
    )
    .await?;

    // Build the fundService calldata.
    let calldata = ITangleWrite::fundServiceCall {
        serviceId: LOCAL_SERVICE_ID,
        amount: fund_amount,
    }
    .abi_encode();

    // Send funding transaction with native ETH.
    use blueprint_sdk::alloy::primitives::TxKind;
    use blueprint_sdk::alloy::rpc::types::TransactionRequest;

    let tx = TransactionRequest {
        from: Some(service_owner),
        to: Some(TxKind::Call(harness.tangle_contract)),
        input: blueprint_sdk::alloy::rpc::types::TransactionInput::new(
            Bytes::from(calldata),
        ),
        value: Some(fund_amount),
        gas: Some(500_000),
        gas_price: Some(1),
        ..Default::default()
    };

    let tx_hash: String = ws_provider
        .raw_request(Cow::Borrowed("eth_sendTransaction"), json!([tx]))
        .await
        .context("fundService tx")?;

    // Mine the transaction.
    anvil_rpc(&ws_provider, "anvil_mine", json!(["0x1"])).await?;

    // Wait for receipt.
    loop {
        let receipt: serde_json::Value = ws_provider
            .raw_request(
                Cow::Borrowed("eth_getTransactionReceipt"),
                json!([tx_hash]),
            )
            .await?;
        if !receipt.is_null() {
            let status = receipt["status"].as_str().unwrap_or("0x0");
            assert_eq!(status, "0x1", "fundService should succeed: {receipt}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    eprintln!("Funded escrow with {fund_amount}");

    // Stop impersonation.
    anvil_rpc(
        &ws_provider,
        "anvil_stopImpersonatingAccount",
        json!([format!("{:#x}", service_owner)]),
    )
    .await?;

    // Now check_escrow should return true.
    let config = watchdog_config(rpc_url, harness.tangle_contract);
    let result = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert!(result, "escrow should be sufficient after funding");

    Ok(())
}

/// Run watchdog_tick against real Anvil and verify it doesn't panic or error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_watchdog_tick_real_rpc() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set BILLING_ANVIL=1 to run");
        return Ok(());
    }

    let _guard = HARNESS_LOCK.lock().await;

    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let rpc_url = harness.http_endpoint().as_str();
    let config = watchdog_config(rpc_url, harness.tangle_contract);

    // Run a single watchdog tick — should complete without panic.
    // This exercises the full pipeline: RPC → ABI decode → comparison → counter update.
    billing::escrow_watchdog_tick(&config).await;

    eprintln!("watchdog_tick completed successfully against real Anvil");

    Ok(())
}
