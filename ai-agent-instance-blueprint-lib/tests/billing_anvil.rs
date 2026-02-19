//! Anvil integration tests for billing lifecycle.
//!
//! Spins up a real Anvil instance with the full tnt-core stack deployed,
//! then tests the escrow watchdog against actual on-chain state.
//! No mocked JSON-RPC — all calls hit the deployed Tangle contract.
//!
//! Uses `anvil_setStorageAt` to manipulate subscription rate and escrow
//! balance directly, enabling tests for insufficient escrow, threshold
//! deprovision, and recovery flows on real contracts.
//!
//! Requires Docker (testcontainers) and the tnt-core broadcast artifacts.
//! Gated behind `BILLING_ANVIL=1` env var (slow, needs Docker).

#![cfg(feature = "billing")]

use ai_agent_instance_blueprint_lib::billing::{
    self, EscrowWatchdog, EscrowWatchdogConfig, ITangleRead, WatchdogTickResult,
};
use anyhow::{Context, Result};
use blueprint_anvil_testing_utils::{
    missing_tnt_core_artifacts, TangleHarness, LOCAL_BLUEPRINT_ID, LOCAL_SERVICE_ID,
};
use blueprint_sdk::alloy::primitives::{keccak256, Address, B256, U256};
use blueprint_sdk::alloy::providers::{Provider, ProviderBuilder};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::borrow::Cow;
use tokio::sync::Mutex as AsyncMutex;

static HARNESS_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

// ─────────────────────────────────────────────────────────────────────────────
// Storage slot helpers (tnt-core TangleStorage.sol layout)
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the base storage slot for a `mapping(uint64 => T)` entry.
/// Follows Solidity's keccak256(abi.encode(key, slotNumber)) convention.
fn mapping_base_slot(key: u64, mapping_slot: u64) -> B256 {
    let mut data = [0u8; 64];
    // key: uint256(key), big-endian, left-padded
    data[24..32].copy_from_slice(&key.to_be_bytes());
    // mapping slot: uint256(mapping_slot), big-endian, left-padded
    data[56..64].copy_from_slice(&mapping_slot.to_be_bytes());
    keccak256(data)
}

/// Offset a base slot by a struct field offset.
fn struct_field_slot(base: B256, offset: u64) -> U256 {
    U256::from_be_bytes(base.0) + U256::from(offset)
}

/// Set a storage slot value on a contract via Anvil RPC.
async fn set_storage<P: Provider>(
    provider: &P,
    contract: Address,
    slot: U256,
    value: U256,
) -> Result<()> {
    provider
        .raw_request::<_, Value>(
            Cow::Borrowed("anvil_setStorageAt"),
            json!([
                format!("{:#x}", contract),
                format!("{:#066x}", slot),
                format!("{:#066x}", value),
            ]),
        )
        .await
        .context("anvil_setStorageAt")?;
    Ok(())
}

/// Mine a block so storage changes take effect for subsequent view calls.
async fn mine_block<P: Provider>(provider: &P) -> Result<()> {
    provider
        .raw_request::<_, Value>(Cow::Borrowed("anvil_mine"), json!(["0x1"]))
        .await
        .context("anvil_mine")?;
    Ok(())
}

/// Discover the actual Solidity storage mapping slot for `_blueprintConfigs`
/// by probing candidate slots and checking if `getBlueprintConfig` reads back
/// the marker value from `subscriptionRate` (struct offset +1).
async fn discover_blueprint_configs_slot<P: Provider>(
    provider: &P,
    contract: Address,
) -> Result<u64> {
    let marker = U256::from(0xDEADBEEF_42u64);
    let read = ITangleRead::new(contract, provider);

    for candidate in 0..=80 {
        let base = mapping_base_slot(LOCAL_BLUEPRINT_ID, candidate);
        let rate_slot = struct_field_slot(base, 1);

        set_storage(provider, contract, rate_slot, marker).await?;

        let config = read
            .getBlueprintConfig(LOCAL_BLUEPRINT_ID)
            .call()
            .await;

        // Reset before checking result
        set_storage(provider, contract, rate_slot, U256::ZERO).await?;

        if let Ok(cfg) = config {
            if cfg.subscriptionRate == marker {
                eprintln!("Discovered _blueprintConfigs mapping slot = {candidate}");
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!("Could not find _blueprintConfigs mapping slot in 0..80")
}

/// Discover the actual Solidity storage mapping slot for `_serviceEscrows`
/// by probing candidate slots and checking if `getServiceEscrow` reads back
/// the marker value from `balance` (struct offset +1).
async fn discover_service_escrows_slot<P: Provider>(
    provider: &P,
    contract: Address,
) -> Result<u64> {
    let marker = U256::from(0xCAFEBABE_42u64);
    let read = ITangleRead::new(contract, provider);

    for candidate in 0..=80 {
        let base = mapping_base_slot(LOCAL_SERVICE_ID, candidate);
        let balance_slot = struct_field_slot(base, 1);

        set_storage(provider, contract, balance_slot, marker).await?;

        let escrow = read
            .getServiceEscrow(LOCAL_SERVICE_ID)
            .call()
            .await;

        // Reset
        set_storage(provider, contract, balance_slot, U256::ZERO).await?;

        if let Ok(esc) = escrow {
            if esc.balance == marker {
                eprintln!("Discovered _serviceEscrows mapping slot = {candidate}");
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!("Could not find _serviceEscrows mapping slot in 0..80")
}

/// Cached mapping slot numbers discovered at runtime from the deployed contract.
struct StorageSlots {
    blueprint_configs: u64,
    service_escrows: u64,
}

async fn discover_slots<P: Provider>(
    provider: &P,
    contract: Address,
) -> Result<StorageSlots> {
    let blueprint_configs = discover_blueprint_configs_slot(provider, contract).await?;
    let service_escrows = discover_service_escrows_slot(provider, contract).await?;
    Ok(StorageSlots {
        blueprint_configs,
        service_escrows,
    })
}

/// Storage slot for `_blueprintConfigs[blueprintId].subscriptionRate`.
fn subscription_rate_slot(blueprint_id: u64, mapping_slot: u64) -> U256 {
    let base = mapping_base_slot(blueprint_id, mapping_slot);
    struct_field_slot(base, 1)
}

/// Storage slot for `_serviceEscrows[serviceId].balance`.
fn escrow_balance_slot(service_id: u64, mapping_slot: u64) -> U256 {
    let base = mapping_base_slot(service_id, mapping_slot);
    struct_field_slot(base, 1)
}

// ─────────────────────────────────────────────────────────────────────────────
// Harness helpers
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

fn watchdog_config(
    rpc_url: &str,
    tangle_contract: Address,
    max_failures: u32,
) -> EscrowWatchdogConfig {
    EscrowWatchdogConfig {
        tangle_contract,
        http_rpc_endpoint: rpc_url.to_string(),
        service_id: LOCAL_SERVICE_ID,
        blueprint_id: LOCAL_BLUEPRINT_ID,
        check_interval_secs: 1,
        max_consecutive_failures: max_failures,
        low_balance_multiplier: 0, // disable low-balance warnings in Anvil tests
        deprovision_grace_period_secs: 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: ABI roundtrip against real contracts
// ─────────────────────────────────────────────────────────────────────────────

/// Verify check_escrow can talk to the real deployed Tangle contract.
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
    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);

    let status = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    eprintln!("check_escrow result: sufficient={}, balance={}, rate={}", status.sufficient, status.balance, status.rate);
    Ok(())
}

/// Read escrow and config directly, verify check_escrow agrees.
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

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let status = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let expected = if bp_config.subscriptionRate == U256::ZERO {
        true
    } else {
        escrow.balance >= bp_config.subscriptionRate
    };

    assert_eq!(status.sufficient, expected, "check_escrow should match raw comparison");
    assert_eq!(status.balance, escrow.balance, "balance should match raw read");
    assert_eq!(status.rate, bp_config.subscriptionRate, "rate should match raw read");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Storage manipulation to test insufficient escrow
// ─────────────────────────────────────────────────────────────────────────────

/// Discover storage slots and verify we can manipulate subscriptionRate + escrow balance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_storage_slot_discovery_and_verification() -> Result<()> {
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

    let slots = discover_slots(&provider, harness.tangle_contract).await?;
    eprintln!(
        "Discovered slots: blueprintConfigs={}, serviceEscrows={}",
        slots.blueprint_configs, slots.service_escrows
    );

    // Verify: set subscriptionRate, read it back
    let rate_value = U256::from(1_000_000u64);
    set_storage(
        &provider,
        harness.tangle_contract,
        subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs),
        rate_value,
    )
    .await?;
    mine_block(&provider).await?;

    let contract = ITangleRead::new(harness.tangle_contract, &provider);
    let bp_config = contract
        .getBlueprintConfig(LOCAL_BLUEPRINT_ID)
        .call()
        .await
        .context("getBlueprintConfig")?;

    assert_eq!(
        bp_config.subscriptionRate, rate_value,
        "subscriptionRate should match what we set via storage"
    );
    Ok(())
}

/// Set rate > 0, leave balance = 0 → check_escrow returns false.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_check_escrow_insufficient_real_rpc() -> Result<()> {
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
    let slots = discover_slots(&provider, harness.tangle_contract).await?;

    // Set subscriptionRate to 1 ETH
    set_storage(
        &provider,
        harness.tangle_contract,
        subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs),
        U256::from(1_000_000_000_000_000_000u128),
    )
    .await?;

    // Zero out escrow balance
    set_storage(
        &provider,
        harness.tangle_contract,
        escrow_balance_slot(LOCAL_SERVICE_ID, slots.service_escrows),
        U256::ZERO,
    )
    .await?;

    mine_block(&provider).await?;

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let status = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    assert!(
        !status.sufficient,
        "check_escrow should return false when balance=0 and rate>0"
    );
    assert_eq!(status.balance, U256::ZERO);
    eprintln!("Confirmed: check_escrow returns false for depleted escrow on real RPC");
    Ok(())
}

/// Set rate > 0 and balance > rate → check_escrow returns true.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_check_escrow_sufficient_after_storage_set() -> Result<()> {
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
    let slots = discover_slots(&provider, harness.tangle_contract).await?;
    let one_eth = U256::from(1_000_000_000_000_000_000u128);

    set_storage(
        &provider,
        harness.tangle_contract,
        subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs),
        one_eth,
    )
    .await?;

    set_storage(
        &provider,
        harness.tangle_contract,
        escrow_balance_slot(LOCAL_SERVICE_ID, slots.service_escrows),
        one_eth * U256::from(10),
    )
    .await?;

    mine_block(&provider).await?;

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let status = billing::check_escrow(&config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    assert!(
        status.sufficient,
        "check_escrow should return true when balance(10 ETH) >= rate(1 ETH)"
    );
    assert_eq!(status.balance, one_eth * U256::from(10));
    assert_eq!(status.rate, one_eth);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Watchdog tick + counter logic against real Anvil state
// ─────────────────────────────────────────────────────────────────────────────

/// EscrowWatchdog tick returns Insufficient against real depleted escrow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_watchdog_tick_insufficient_real_rpc() -> Result<()> {
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
    let slots = discover_slots(&provider, harness.tangle_contract).await?;

    let one_eth = U256::from(1_000_000_000_000_000_000u128);
    set_storage(
        &provider,
        harness.tangle_contract,
        subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs),
        one_eth,
    )
    .await?;
    set_storage(
        &provider,
        harness.tangle_contract,
        escrow_balance_slot(LOCAL_SERVICE_ID, slots.service_escrows),
        U256::ZERO,
    )
    .await?;
    mine_block(&provider).await?;

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
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
    Ok(())
}

/// Three ticks with depleted escrow → DeprovisionRequired on third tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_watchdog_tick_threshold_real_rpc() -> Result<()> {
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
    let slots = discover_slots(&provider, harness.tangle_contract).await?;

    let one_eth = U256::from(1_000_000_000_000_000_000u128);
    set_storage(
        &provider,
        harness.tangle_contract,
        subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs),
        one_eth,
    )
    .await?;
    set_storage(
        &provider,
        harness.tangle_contract,
        escrow_balance_slot(LOCAL_SERVICE_ID, slots.service_escrows),
        U256::ZERO,
    )
    .await?;
    mine_block(&provider).await?;

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let watchdog = EscrowWatchdog::new(config);

    let r1 = watchdog.tick().await;
    assert!(matches!(r1, WatchdogTickResult::Insufficient { consecutive: 1, threshold: 3 }));

    let r2 = watchdog.tick().await;
    assert!(matches!(r2, WatchdogTickResult::Insufficient { consecutive: 2, threshold: 3 }));

    let r3 = watchdog.tick().await;
    assert_eq!(r3, WatchdogTickResult::DeprovisionRequired { consecutive: 3 });
    eprintln!("Confirmed: 3 consecutive failures against real RPC → DeprovisionRequired");
    Ok(())
}

/// Full lifecycle: fund → tick (sufficient) → drain → tick x3 (deprovision) → refund → tick (recovery).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_watchdog_full_lifecycle_real_rpc() -> Result<()> {
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
    let slots = discover_slots(&provider, harness.tangle_contract).await?;
    let one_eth = U256::from(1_000_000_000_000_000_000u128);

    let rate_slot = subscription_rate_slot(LOCAL_BLUEPRINT_ID, slots.blueprint_configs);
    let balance_slot = escrow_balance_slot(LOCAL_SERVICE_ID, slots.service_escrows);

    // Set subscription rate to 1 ETH
    set_storage(&provider, harness.tangle_contract, rate_slot, one_eth).await?;

    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let watchdog = EscrowWatchdog::new(config);

    // ── Phase 1: Escrow funded (10 ETH) → Sufficient ──
    set_storage(&provider, harness.tangle_contract, balance_slot, one_eth * U256::from(10)).await?;
    mine_block(&provider).await?;

    let r = watchdog.tick().await;
    assert_eq!(r, WatchdogTickResult::Sufficient { previous_failures: 0 });
    eprintln!("Phase 1: funded escrow → Sufficient");

    // ── Phase 2: Drain escrow to 0 → 3 ticks → DeprovisionRequired ──
    set_storage(&provider, harness.tangle_contract, balance_slot, U256::ZERO).await?;
    mine_block(&provider).await?;

    let r = watchdog.tick().await;
    assert!(matches!(r, WatchdogTickResult::Insufficient { consecutive: 1, .. }));

    let r = watchdog.tick().await;
    assert!(matches!(r, WatchdogTickResult::Insufficient { consecutive: 2, .. }));

    let r = watchdog.tick().await;
    assert_eq!(r, WatchdogTickResult::DeprovisionRequired { consecutive: 3 });
    eprintln!("Phase 2: drained escrow → 3 failures → DeprovisionRequired");

    // ── Phase 3: Refund escrow → Sufficient (recovery) ──
    set_storage(&provider, harness.tangle_contract, balance_slot, one_eth * U256::from(5)).await?;
    mine_block(&provider).await?;

    let r = watchdog.tick().await;
    assert_eq!(r, WatchdogTickResult::Sufficient { previous_failures: 3 });
    assert_eq!(watchdog.failure_count(), 0, "recovery must reset counter");
    eprintln!("Phase 3: refunded escrow → recovery, counter reset to 0");

    // ── Phase 4: Drain again → fresh counter starts from 1 ──
    set_storage(&provider, harness.tangle_contract, balance_slot, U256::ZERO).await?;
    mine_block(&provider).await?;

    let r = watchdog.tick().await;
    assert!(matches!(r, WatchdogTickResult::Insufficient { consecutive: 1, .. }));
    eprintln!("Phase 4: drained again → fresh counter starts at 1");

    Ok(())
}

/// Single watchdog tick against real Anvil — verify it doesn't panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_watchdog_tick_no_panic() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set BILLING_ANVIL=1 to run");
        return Ok(());
    }
    let _guard = HARNESS_LOCK.lock().await;
    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let rpc_url = harness.http_endpoint().as_str();
    let config = watchdog_config(rpc_url, harness.tangle_contract, 3);
    let watchdog = EscrowWatchdog::new(config);

    let result = watchdog.tick().await;
    eprintln!("watchdog.tick() result: {result:?}");

    // With default seeded state (rate=0), should always be Sufficient
    assert!(matches!(result, WatchdogTickResult::Sufficient { .. }));
    Ok(())
}
