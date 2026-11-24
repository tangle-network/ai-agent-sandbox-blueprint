//! Anvil integration tests for the service extension watcher (issue #239).
//!
//! Chain-only: boots the Tangle harness (tnt-core on local Anvil), rewrites
//! the on-chain paid window (`createdAt`/`ttl`) for `LOCAL_SERVICE_ID`
//! through `anvil_setStorageAt` on the discovered `_services` mapping slot,
//! and drives `ServiceExtensionWatcher::tick` against synthetic sandbox
//! records in an isolated `BLUEPRINT_STATE_DIR`. No Docker, sidecar, or job
//! router is involved. Deletion after the paid window elapses is the
//! reaper's contract and stays covered by its own suites.
//!
//! Slow and needs the tnt-core broadcast artifacts; self-skips unless
//! `SERVICE_EXTENSION_ANVIL=1` is set.

use ai_agent_sandbox_blueprint_lib::service_extension::{
    ITangleServiceReader, ServiceExtensionConfig, ServiceExtensionWatcher,
};
use anyhow::Result;
use blueprint_anvil_testing_utils::{LOCAL_SERVICE_ID, TangleHarness, missing_tnt_core_artifacts};
use blueprint_sdk::alloy::primitives::{Address, B256, U256, keccak256};
use blueprint_sdk::alloy::providers::{Provider, ProviderBuilder};
use sandbox_runtime::SandboxRecord;
use sandbox_runtime::runtime::sandboxes;
use sandbox_runtime::util::now_ts;
use serde_json::json;
use std::str::FromStr;
use std::time::Duration;

fn should_run() -> bool {
    std::env::var("SERVICE_EXTENSION_ANVIL").ok().as_deref() == Some("1")
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
            eprintln!("Skipping service-extension Anvil test: {e}");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers (mirror the billing_anvil slot-surgery helpers)
// ─────────────────────────────────────────────────────────────────────────────

/// keccak256(uint256(key) ++ uint256(mapping_slot)) — Solidity mapping base slot.
fn mapping_base_slot(key: u64, mapping_slot: u64) -> B256 {
    let mut data = [0u8; 64];
    data[24..32].copy_from_slice(&key.to_be_bytes());
    data[56..64].copy_from_slice(&mapping_slot.to_be_bytes());
    keccak256(data)
}

/// `base + offset` as a storage slot.
fn field_slot(base: B256, offset: u64) -> B256 {
    B256::from(U256::from(base) + U256::from(offset))
}

async fn set_storage(provider: &impl Provider, contract: Address, slot: B256, value: B256) {
    provider
        .raw_request::<serde_json::Value, bool>(
            "anvil_setStorageAt".into(),
            json!([
                format!("{contract:#x}"),
                format!("{slot:#066x}"),
                format!("{value:#066x}"),
            ]),
        )
        .await
        .expect("anvil_setStorageAt failed");
}

async fn mine_block(provider: &impl Provider) {
    provider
        .raw_request::<serde_json::Value, serde_json::Value>(
            "anvil_mine".into(),
            json!(["0x1"]),
        )
        .await
        .expect("anvil_mine failed");
}

async fn storage_word(provider: &impl Provider, contract: Address, slot: B256) -> B256 {
    let word: String = provider
        .raw_request::<serde_json::Value, String>(
            "eth_getStorageAt".into(),
            json!([format!("{contract:#x}"), format!("{slot:#066x}"), "latest"]),
        )
        .await
        .expect("eth_getStorageAt failed");
    B256::from_str(word.trim_start_matches("0x"))
        .unwrap_or_else(|e| panic!("bad storage word {word}: {e}"))
}

/// Current on-chain `ttl` for `service_id` through the real `getService` view.
async fn service_ttl(provider: &impl Provider, contract: Address, service_id: u64) -> u64 {
    ITangleServiceReader::new(contract, provider)
        .getService(service_id)
        .call()
        .await
        .expect("getService failed")
        .ttl
}

/// Overwrite `Service` slot 1: createdAt[0..8], ttl[8..16], rest zero.
async fn write_paid_window(
    provider: &impl Provider,
    contract: Address,
    slot1: B256,
    created_at: u64,
    ttl: u64,
) {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&created_at.to_be_bytes());
    bytes[8..16].copy_from_slice(&ttl.to_be_bytes());
    set_storage(provider, contract, slot1, B256::from(bytes)).await;
    mine_block(provider).await;
}

/// Find the `_services` mapping slot by patching `ttl` (slot+1, bytes 8..16)
/// with a marker and checking it through the real `getService` view.
async fn discover_services_slot(provider: &impl Provider, contract: Address) -> u64 {
    const MARKER: u64 = 0x1011_1213_1415_1617;
    for slot in 180u64..=200 {
        let slot1 = field_slot(mapping_base_slot(LOCAL_SERVICE_ID, slot), 1);
        let original = storage_word(provider, contract, slot1).await;
        let mut bytes: [u8; 32] = original.into();
        bytes[8..16].copy_from_slice(&MARKER.to_be_bytes());
        set_storage(provider, contract, slot1, B256::from(bytes)).await;
        mine_block(provider).await;
        let found = service_ttl(provider, contract, LOCAL_SERVICE_ID).await == MARKER;
        set_storage(provider, contract, slot1, original).await;
        mine_block(provider).await;
        if found {
            return slot;
        }
    }
    panic!("_services mapping slot not found in 180..=200");
}

/// SandboxRecord has no `Default`; required fields are provided and the
/// serde-defaulted rest is filled from the JSON form.
fn synthetic_record(
    id: &str,
    service_id: u64,
    created_at: u64,
    max_lifetime: u64,
) -> SandboxRecord {
    serde_json::from_value(json!({
        "id": id,
        "container_id": format!("container-{id}"),
        "sidecar_url": "http://127.0.0.1:1",
        "sidecar_port": 1,
        "token": format!("token-{id}"),
        "created_at": created_at,
        "state": "Running",
        "service_id": service_id,
        "max_lifetime_seconds": max_lifetime,
    }))
    .expect("synthetic SandboxRecord")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

/// The watcher raises matching sandboxes' lifetimes to the paid window,
/// leaves other services' sandboxes alone, and never shrinks lifetimes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watcher_extends_and_never_shrinks_local_lifetimes() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set SERVICE_EXTENSION_ANVIL=1 to run");
        return Ok(());
    }
    let Some(harness) = spawn_harness().await? else {
        return Ok(());
    };

    let endpoint = harness.http_endpoint().to_string();
    let contract: Address = harness.tangle_contract;
    let provider = ProviderBuilder::new()
        .connect(endpoint.as_str())
        .await
        .expect("failed to connect provider");

    // Isolated store: env must be set before the first `sandboxes()` call.
    let state_dir = tempfile::tempdir().expect("temp state dir");
    // SAFETY: tests are the only writer of this env var in this process.
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
    }
    let store = sandboxes().expect("sandbox store");

    let services_slot = discover_services_slot(&provider, contract).await;
    let slot1 = field_slot(mapping_base_slot(LOCAL_SERVICE_ID, services_slot), 1);

    let created_at = now_ts() - 500;
    store.insert(
        "ext-target".to_string(),
        synthetic_record("ext-target", LOCAL_SERVICE_ID, created_at, 60),
    )?;
    store.insert(
        "ext-other-service".to_string(),
        synthetic_record("ext-other-service", 9_999, created_at, 60),
    )?;

    let config = ServiceExtensionConfig {
        service_id: LOCAL_SERVICE_ID,
        contract_address: contract,
        endpoint,
        interval: Duration::from_secs(1),
    };
    let watcher = ServiceExtensionWatcher::new(config);

    // Phase 1: paid window created_at + 2000 raises the 60s lifetime to 2000.
    write_paid_window(&provider, contract, slot1, created_at, 2000).await;
    let summary = watcher.tick().await.expect("tick failed");
    assert_eq!(summary.running, 1);
    assert_eq!(summary.extended, 1);
    assert_eq!(
        store.get("ext-target")?.expect("target record").max_lifetime_seconds,
        2000
    );
    assert_eq!(
        store
            .get("ext-other-service")?
            .expect("other-service record")
            .max_lifetime_seconds,
        60,
        "sandboxes of other services must not be touched"
    );

    // Phase 2: a shorter window must never shrink the stored lifetime.
    write_paid_window(&provider, contract, slot1, created_at, 100).await;
    let summary = watcher.tick().await.expect("tick failed");
    assert_eq!(summary.running, 1);
    assert_eq!(summary.extended, 0, "extend-only watcher must not shrink");
    assert_eq!(
        store.get("ext-target")?.expect("target record").max_lifetime_seconds,
        2000
    );

    store.remove("ext-target")?;
    store.remove("ext-other-service")?;
    Ok(())
}
