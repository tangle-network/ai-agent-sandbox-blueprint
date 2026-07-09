//! Ground-truth decode check for the tnt-core 0.19 bindings against the live
//! Tempo deployment. Reproduces the exact call that overran with the stale
//! 0.18 bindings: `get_blueprint_manager(service_id)` (get_service -> get_blueprint).
//!
//! Run explicitly (network + live chain):
//!   cargo test -p ai-agent-sandbox-blueprint-lib --test tempo_decode -- --ignored --nocapture

use blueprint_sdk::alloy::primitives::{address, Address};
use blueprint_sdk::clients::tangle::{TangleClient, TangleClientConfig, TangleSettings};
use blueprint_sdk::crypto::KeyType;
use blueprint_sdk::crypto::k256::K256Ecdsa;
use blueprint_sdk::keystore::backends::Backend;
use blueprint_sdk::keystore::{Keystore, KeystoreConfig};
use reqwest::Url;

const TEMPO_RPC: &str = "https://rpc.moderato.tempo.xyz";
const TANGLE: Address = address!("ff137b9c879c47c28ce389e84501925438ab4cda");
const EXPECTED_MANAGER: Address = address!("506483972499f7b6060d517066eb666ce9c10978");
const SERVICE_ID: u64 = 3;
const BLUEPRINT_ID: u64 = 4;

#[tokio::test]
#[ignore = "hits live Tempo RPC"]
async fn get_blueprint_manager_decodes_on_tempo() {
    let keystore = Keystore::new(KeystoreConfig::new().in_memory(true)).expect("keystore");
    // Read-only view calls do not sign; any well-formed ECDSA key satisfies the
    // client's `first_local::<K256Ecdsa>()` account lookup.
    let secret = K256Ecdsa::generate_with_seed(Some(&[7u8; 32])).expect("generate key");
    keystore.insert::<K256Ecdsa>(&secret).expect("insert key");

    let settings = TangleSettings {
        blueprint_id: BLUEPRINT_ID,
        service_id: Some(SERVICE_ID),
        tangle_contract: TANGLE,
        staking_contract: Address::ZERO,
        status_registry_contract: Address::ZERO,
    };
    let config = TangleClientConfig::new(
        Url::parse(TEMPO_RPC).unwrap(),
        Url::parse(TEMPO_RPC).unwrap(),
        "memory://",
        settings,
    )
    .test_mode(true);

    let client = TangleClient::with_keystore(config, keystore)
        .await
        .expect("build TangleClient");

    // This is the call that failed with "ABI decoding failed: buffer overrun"
    // under tnt-core-bindings 0.18.0.
    let manager = client
        .get_blueprint_manager(SERVICE_ID)
        .await
        .expect("get_blueprint_manager must decode without buffer overrun");

    println!("get_blueprint_manager({SERVICE_ID}) = {manager:?}");
    assert_eq!(
        manager,
        Some(EXPECTED_MANAGER),
        "manager address must match the on-chain BSM"
    );
}
