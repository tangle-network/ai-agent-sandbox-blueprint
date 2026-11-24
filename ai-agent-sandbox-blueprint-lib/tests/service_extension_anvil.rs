//! Anvil integration test for the service extension watcher (issue #239).
//!
//! End-to-end and real: boots the Tangle harness (tnt-core on local Anvil),
//! backdates the chain with `evm_setTime` so a minimum-TTL service (1 hour)
//! still interacts with the reaper's real wall clock in seconds, then drives
//! the production lifecycle:
//!
//! 1. `requestService` + single-operator `approveService` activate a fresh
//!    TTL-bearing service through real transactions.
//! 2. A synthetic firecracker-backend sandbox record is capped by the real
//!    [`clamp_lifetime_to_paid_time`] (the on-chain paid window).
//! 3. `extendServiceFromQuotes` extends the service with an EIP-712 quote
//!    signed by the seeded operator (payment: 1 wei, exact).
//! 4. `ServiceExtensionWatcher::tick` raises the record's lifetime to the
//!    extended paid window; another service's record is untouched.
//! 5. `reaper_tick` (real wall clock): the record survives past its
//!    ORIGINAL deadline and is deleted after the EXTENDED deadline.
//!
//! The record is synthetic (no real Firecracker host requirements), but every
//! lifetime decision — clamp, extension, watcher, reaping — runs through the
//! production code paths. No contract storage is patched.
//!
//! Slow and needs the tnt-core broadcast artifacts; self-skips unless
//! `SERVICE_EXTENSION_ANVIL=1` is set. With the flag set, missing
//! prerequisites fail the test with remediation hints.

use ai_agent_sandbox_blueprint_lib::service_extension::{
    ITangleServiceReader, ServiceExtensionConfig, ServiceExtensionWatcher,
    clamp_lifetime_to_paid_time,
};
use anyhow::Result;
use blueprint_anvil_testing_utils::TangleHarness;
use blueprint_sdk::alloy::primitives::{Address, Bytes, B256, U256, keccak256};
use blueprint_sdk::alloy::providers::{Provider, ProviderBuilder};
use blueprint_sdk::alloy::sol;
use blueprint_sdk::alloy::sol_types::SolCall;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use sandbox_runtime::SandboxRecord;
use sandbox_runtime::reaper::reaper_tick;
use sandbox_runtime::runtime::sandboxes;
use sandbox_runtime::util::now_ts;
use serde_json::json;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

sol! {
    #[sol(rpc)]
    #[allow(missing_docs)]
    interface ITangleActions {
        struct Asset {
            uint8 kind;
            address token;
        }
        struct AssetSecurityCommitment {
            Asset asset;
            uint16 exposureBps;
        }
        struct ResourceCommitment {
            uint8 kind;
            uint64 count;
        }
        struct TeeAttestationCommitment {
            uint8 backend;
            bytes32 expectedMeasurement;
            bytes32 nonceBinding;
            uint64 expiresAt;
        }
        struct QuoteDetails {
            address requester;
            uint64 blueprintId;
            uint64 ttlBlocks;
            uint256 totalCost;
            uint64 timestamp;
            uint64 expiry;
            uint8 confidentiality;
            uint8 operation;
            uint64 serviceId;
            AssetSecurityCommitment[] securityCommitments;
            ResourceCommitment[] resourceCommitments;
        }
        struct SignedQuote {
            QuoteDetails details;
            bytes signature;
            address operator;
        }
        struct ApprovalParams {
            uint64 requestId;
            AssetSecurityCommitment[] securityCommitments;
            uint256[4] blsPubkey;
            uint256[2] blsPopSignature;
            TeeAttestationCommitment[] teeCommitments;
        }
        function requestService(
            uint64 blueprintId,
            address[] calldata operators,
            bytes calldata config,
            address[] calldata permittedCallers,
            uint64 ttl,
            address paymentToken,
            uint256 paymentAmount,
            uint8 confidentiality
        ) external payable returns (uint64 requestId);
        function approveService(ApprovalParams calldata params) external;
        function extendServiceFromQuotes(
            uint64 serviceId,
            SignedQuote[] calldata quotes,
            uint64 additionalTtl
        ) external payable;
        event ServiceExtended(uint64 indexed serviceId, uint64 oldTtl, uint64 newTtl, uint256 payment);
    }
}

/// Blueprint 0 is registered by the seeded local-testnet deployment.
const BLUEPRINT_ID: u64 = 0;
/// `ProtocolConfig.MIN_SERVICE_TTL`: one hour of paid runtime.
const SERVICE_TTL: u64 = 3_600;
/// Extension purchased through `extendServiceFromQuotes`.
const ADDITIONAL_TTL: u64 = 40;
/// Deliberately above the remaining paid window, so the clamp must bite.
const REQUESTED_LIFETIME: u64 = 7_200;
/// Chain backdate: activation lands ~10s before the original deadline in
/// real time, keeping the whole lifecycle observable in seconds.
const BACKDATE_SECS: u64 = 3_590;
/// The decoy record outlives the whole test.
const DECOY_LIFETIME: u64 = 600;
/// Some service the watcher is not configured for.
const OTHER_SERVICE_ID: u64 = 9_999;
/// Quote freshness margin (`expiry - timestamp`).
const QUOTE_VALIDITY: u64 = 86_400;
/// `Types.ConfidentialityPolicy.Any`.
const CONFIDENTIALITY_ANY: u8 = 0;
/// `Types.QuoteOperation.Extend`.
const QUOTE_OPERATION_EXTEND: u8 = 1;
/// `Types.ServiceStatus.Active`.
const SERVICE_STATUS_ACTIVE: u8 = 1;
/// Anvil dev account #0: the requester and service owner.
const DEPLOYER: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cffFb92266";
/// Anvil dev account #1: the seeded operator that signs the extension quote.
const OPERATOR: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";
const OPERATOR_KEY: &str = "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

fn should_run() -> bool {
    std::env::var("SERVICE_EXTENSION_ANVIL").ok().as_deref() == Some("1")
}

/// Fail hard when the flag asks for this suite but the environment cannot
/// provide it: a silent skip would let regressions through the gate.
async fn spawn_harness() -> TangleHarness {
    match TangleHarness::builder()
        .include_anvil_logs(false)
        .seed_from_broadcast(true)
        .spawn()
        .await
    {
        Ok(harness) => harness,
        Err(e) => panic!(
            "SERVICE_EXTENSION_ANVIL=1 but the Tangle harness failed to start: {e}. \
             Run scripts/fetch-localtestnet-fixtures.sh to fetch the tnt-core \
             broadcast artifacts and ensure Docker is running, then re-run."
        ),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_secs()
}

/// Sleep until the wall clock passes `target` (no-op when already past).
async fn sleep_until(target: u64) {
    let now = unix_now();
    if target > now {
        tokio::time::sleep(Duration::from_secs(target - now)).await;
    }
}

/// Left-padded EIP-712 word for an address.
fn word_address(value: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(value.as_slice());
    word
}

/// Left-padded EIP-712 word for a uint.
fn word_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

/// Left-padded EIP-712 word for a uint8.
fn word_u8(value: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = value;
    word
}

/// EIP-712 word for a uint256.
fn word_u256(value: U256) -> [u8; 32] {
    value.to_be_bytes()
}

/// keccak256 of the empty string: the EIP-712 hash of an empty array.
const EMPTY_ARRAY_WORD: B256 = B256::new([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
]);

/// Mirrors `SignatureLib.QUOTE_TYPEHASH` (struct type string plus nested
/// struct definitions, as encoded on-chain).
const QUOTE_TYPE_STRING: &str = "QuoteDetails(address requester,uint64 blueprintId,uint64 ttlBlocks,uint256 totalCost,uint64 timestamp,uint64 expiry,uint8 confidentiality,uint8 operation,uint64 serviceId,AssetSecurityCommitment[] securityCommitments,ResourceCommitment[] resourceCommitments)Asset(uint8 kind,address token)AssetSecurityCommitment(Asset asset,uint16 exposureBps)ResourceCommitment(uint8 kind,uint64 count)";

/// EIP-712 digest of an extension quote, mirroring
/// `SignatureLib.computeDomainSeparator("TangleQuote", "1", contract)` and
/// `SignatureLib.hashQuote`.
fn quote_digest(details: &ITangleActions::QuoteDetails, chain_id: u64, verifier: Address) -> B256 {
    let domain_type_hash = keccak256(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    let mut domain = Vec::with_capacity(5 * 32);
    domain.extend_from_slice(domain_type_hash.as_slice());
    domain.extend_from_slice(keccak256(b"TangleQuote").as_slice());
    domain.extend_from_slice(keccak256(b"1").as_slice());
    domain.extend_from_slice(&word_u256(U256::from(chain_id)));
    domain.extend_from_slice(&word_address(verifier));
    let domain_separator = keccak256(&domain);

    let mut body = Vec::with_capacity(12 * 32);
    body.extend_from_slice(keccak256(QUOTE_TYPE_STRING.as_bytes()).as_slice());
    body.extend_from_slice(&word_address(details.requester));
    body.extend_from_slice(&word_u64(details.blueprintId));
    body.extend_from_slice(&word_u64(details.ttlBlocks));
    body.extend_from_slice(&word_u256(details.totalCost));
    body.extend_from_slice(&word_u64(details.timestamp));
    body.extend_from_slice(&word_u64(details.expiry));
    body.extend_from_slice(&word_u8(details.confidentiality));
    body.extend_from_slice(&word_u8(details.operation));
    body.extend_from_slice(&word_u64(details.serviceId));
    body.extend_from_slice(EMPTY_ARRAY_WORD.as_slice()); // securityCommitments == []
    body.extend_from_slice(EMPTY_ARRAY_WORD.as_slice()); // resourceCommitments == []
    let struct_hash = keccak256(&body);

    let mut signing = Vec::with_capacity(2 + 32 + 32);
    signing.push(0x19);
    signing.push(0x01);
    signing.extend_from_slice(domain_separator.as_slice());
    signing.extend_from_slice(struct_hash.as_slice());
    keccak256(&signing)
}

/// Sign `digest` with `key_hex`, brute-forcing the recovery id so the result
/// verifiably recovers to `expected`. Returns r || s || v (v in {27, 28}).
fn sign_as(digest: &B256, key_hex: &str, expected: Address) -> Vec<u8> {
    let mut secret = [0u8; 32];
    for (i, pair) in key_hex.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).expect("hex digit") as u8;
        let lo = (pair[1] as char).to_digit(16).expect("hex digit") as u8;
        secret[i] = (hi << 4) | lo;
    }
    let signing = SigningKey::from_slice(&secret).expect("valid operator key");
    let raw: Signature = signing.sign_prehash(digest.as_slice()).expect("sign digest");
    let normalized = raw.normalize_s().unwrap_or(raw);
    for y_odd in [false, true] {
        let recovery = RecoveryId::new(y_odd, false);
        let recovered =
            VerifyingKey::recover_from_prehash(digest.as_slice(), &normalized, recovery)
                .expect("recover from prehash");
        let point = recovered.to_encoded_point(false);
        let hashed = keccak256(&point.as_bytes()[1..]);
        let addr = Address::from_slice(&hashed.as_slice()[12..]);
        if addr == expected {
            let mut signature = Vec::with_capacity(65);
            signature.extend_from_slice(&normalized.to_bytes());
            signature.push(27 + u8::from(y_odd));
            return signature;
        }
    }
    panic!("no recovery id reproduces the expected signer {expected:#x}");
}

/// Lowercase `0x`-prefixed hex of `bytes`.
fn hex_data(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + 2 * bytes.len());
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Send a raw transaction from an unlocked Anvil dev account: pre-flight
/// `eth_call` to surface revert reasons, then `eth_sendTransaction` with an
/// explicit gas price (mirroring the harness seeding).
async fn send_from(
    provider: &impl Provider,
    from: Address,
    to: Address,
    data: &[u8],
    value: u128,
) -> B256 {
    let tx = json!({
        "from": format!("{from:#x}"),
        "to": format!("{to:#x}"),
        "data": hex_data(data),
        "gas": "0x7a1200",
        "gasPrice": "0x1",
        "value": format!("{value:#x}"),
    });
    if let Err(e) = provider
        .raw_request::<serde_json::Value, serde_json::Value>(
            "eth_call".into(),
            json!([tx.clone(), "latest"]),
        )
        .await
    {
        panic!("transaction from {from:#x} would revert: {e}");
    }
    let hash: String = provider
        .raw_request::<serde_json::Value, String>("eth_sendTransaction".into(), json!([tx]))
        .await
        .expect("eth_sendTransaction failed");
    B256::from_str(hash.trim_start_matches("0x"))
        .unwrap_or_else(|e| panic!("bad tx hash {hash}: {e}"))
}

/// Poll for the receipt and require status 0x1.
async fn await_receipt(provider: &impl Provider, hash: B256) -> serde_json::Value {
    for _ in 0..300 {
        let receipt = provider
            .raw_request::<serde_json::Value, Option<serde_json::Value>>(
                "eth_getTransactionReceipt".into(),
                json!([format!("{hash:#x}")]),
            )
            .await
            .expect("eth_getTransactionReceipt failed");
        if let Some(receipt) = receipt {
            assert_eq!(
                receipt["status"], "0x1",
                "transaction {hash:#x} reverted: {receipt}"
            );
            return receipt;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no receipt for transaction {hash:#x}");
}

async fn latest_block_timestamp(provider: &impl Provider) -> u64 {
    let block = provider
        .raw_request::<serde_json::Value, serde_json::Value>(
            "eth_getBlockByNumber".into(),
            json!(["latest", false]),
        )
        .await
        .expect("eth_getBlockByNumber failed");
    let ts = block["timestamp"].as_str().expect("block timestamp");
    u64::from_str_radix(ts.trim_start_matches("0x"), 16).expect("block timestamp hex")
}

/// SandboxRecord has no `Default`; required fields are provided and the
/// serde-defaulted rest is filled from the JSON form. The firecracker
/// metadata routes deletion through the production backend path, which is
/// idempotent for the synthetic VM id.
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
        "metadata_json": "{\"runtime_backend\":\"firecracker\"}",
    }))
    .expect("synthetic SandboxRecord")
}

/// The real chain extension raises the local lifetime, and the real reaper
/// honors it: the sandbox survives past the original deadline and is deleted
/// after the extended one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_extension_extends_and_reaper_honors_extended_deadline() -> Result<()> {
    if !should_run() {
        eprintln!("Skipping: set SERVICE_EXTENSION_ANVIL=1 to run");
        return Ok(());
    }
    let harness = spawn_harness().await;

    let endpoint = harness.http_endpoint().to_string();
    let contract: Address = harness.tangle_contract;
    let provider = ProviderBuilder::new()
        .connect(endpoint.as_str())
        .await
        .expect("failed to connect provider");
    let deployer = Address::from_str(DEPLOYER).expect("deployer address");
    let operator = Address::from_str(OPERATOR).expect("operator address");
    let actions = ITangleActions::new(contract, &provider);
    let reader = ITangleServiceReader::new(contract, &provider);

    // The clamp and watcher resolve the contract through the environment, and
    // the store must be isolated before the first `sandboxes()` call.
    let state_dir = tempfile::tempdir().expect("temp state dir");
    // SAFETY: tests are the only writer of these env vars in this process.
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
        std::env::set_var("TANGLE_CONTRACT_ADDRESS", format!("{contract:#x}"));
        std::env::set_var("HTTP_RPC_ENDPOINT", &endpoint);
    }
    let store = sandboxes().expect("sandbox store");

    // Backdate the chain: a service activated at `now - 3590` reaches its
    // original deadline ~10s and its extended deadline ~50s later in real
    // time, so the reaper's wall clock observes both sides of the extension.
    let target_activation = unix_now().saturating_sub(BACKDATE_SECS);
    provider
        .raw_request::<serde_json::Value, serde_json::Value>(
            "evm_setTime".into(),
            json!([format!("{target_activation:#x}")]),
        )
        .await
        .expect("evm_setTime failed");

    // ── Real service creation: request, then single-operator approval ──
    let service_id = actions
        .requestService(
            BLUEPRINT_ID,
            vec![operator],
            Bytes::new(),
            vec![],
            SERVICE_TTL,
            Address::ZERO,
            U256::ZERO,
            CONFIDENTIALITY_ANY,
        )
        .call()
        .await
        .expect("simulate requestService");
    let request = ITangleActions::requestServiceCall {
        blueprintId: BLUEPRINT_ID,
        operators: vec![operator],
        config: Bytes::new(),
        permittedCallers: vec![],
        ttl: SERVICE_TTL,
        paymentToken: Address::ZERO,
        paymentAmount: U256::ZERO,
        confidentiality: CONFIDENTIALITY_ANY,
    };
    let hash = send_from(&provider, deployer, contract, &request.abi_encode(), 0).await;
    await_receipt(&provider, hash).await;

    let approval = ITangleActions::ApprovalParams {
        requestId: service_id,
        securityCommitments: vec![],
        blsPubkey: [U256::ZERO; 4],
        blsPopSignature: [U256::ZERO; 2],
        teeCommitments: vec![],
    };
    let hash = send_from(
        &provider,
        operator,
        contract,
        &ITangleActions::approveServiceCall { params: approval }.abi_encode(),
        0,
    )
    .await;
    await_receipt(&provider, hash).await;

    let service = reader
        .getService(service_id)
        .call()
        .await
        .expect("getService failed");
    assert_eq!(service.owner, deployer, "deployer must own the service");
    assert_eq!(service.ttl, SERVICE_TTL);
    assert_eq!(
        service.status, SERVICE_STATUS_ACTIVE,
        "single-operator approval must activate the service"
    );
    let original_deadline = service.createdAt + service.ttl;
    let activation_age = unix_now().saturating_sub(service.createdAt);
    assert!(
        (BACKDATE_SECS - 100..=BACKDATE_SECS + 100).contains(&activation_age),
        "activation must be backdated ~{BACKDATE_SECS}s (age {activation_age}s; \
         evm_setTime ineffective?)"
    );

    // ── Local lifetime capped by the real remaining paid window ──
    let clamped = clamp_lifetime_to_paid_time(service_id, REQUESTED_LIFETIME)
        .await
        .expect("clamp must succeed while the window is fully paid");
    let expected_remaining = original_deadline.saturating_sub(unix_now()) as i64;
    assert!(
        (clamped as i64 - expected_remaining).abs() <= 10,
        "clamp {clamped} must track the remaining paid window ~{expected_remaining}"
    );
    assert!(
        clamped < REQUESTED_LIFETIME,
        "clamp must cap the over-requested lifetime"
    );
    let record_created = now_ts();
    store.insert(
        "ext-target".to_string(),
        synthetic_record("ext-target", service_id, record_created, clamped),
    )?;
    store.insert(
        "ext-other-service".to_string(),
        synthetic_record("ext-other-service", OTHER_SERVICE_ID, record_created, DECOY_LIFETIME),
    )?;

    // ── Real extension with an EIP-712 quote signed by the operator ──
    let chain_now = latest_block_timestamp(&provider).await;
    let details = ITangleActions::QuoteDetails {
        requester: deployer,
        blueprintId: BLUEPRINT_ID,
        ttlBlocks: ADDITIONAL_TTL, // extension quotes price the additional window
        totalCost: U256::from(1u64),
        timestamp: chain_now,
        expiry: chain_now + QUOTE_VALIDITY,
        confidentiality: CONFIDENTIALITY_ANY, // must match the service policy
        operation: QUOTE_OPERATION_EXTEND,
        serviceId: service_id,
        securityCommitments: vec![],
        resourceCommitments: vec![],
    };
    let chain_id = provider.get_chain_id().await.expect("chain id");
    let digest = quote_digest(&details, chain_id, contract);
    let quote = ITangleActions::SignedQuote {
        details,
        signature: Bytes::from(sign_as(&digest, OPERATOR_KEY, operator)),
        operator,
    };
    let hash = send_from(
        &provider,
        deployer,
        contract,
        &ITangleActions::extendServiceFromQuotesCall {
            serviceId: service_id,
            quotes: vec![quote],
            additionalTtl: ADDITIONAL_TTL,
        }
        .abi_encode(),
        1, // exact quote totalCost
    )
    .await;
    let receipt = await_receipt(&provider, hash).await;

    let topic = format!("{:#x}", keccak256(b"ServiceExtended(uint64,uint64,uint256)"));
    let emitted = receipt["logs"]
        .as_array()
        .expect("receipt logs")
        .iter()
        .filter(|log| log["topics"][0].as_str() == Some(topic.as_str()))
        .count();
    assert_eq!(emitted, 1, "ServiceExtended must be emitted once: {receipt}");
    let extended = reader
        .getService(service_id)
        .call()
        .await
        .expect("getService after extension failed");
    assert_eq!(extended.ttl, SERVICE_TTL + ADDITIONAL_TTL);
    let extended_deadline = extended.createdAt + extended.ttl;

    // ── Watcher raises the local lifetime to the extended window ──
    let watcher = ServiceExtensionWatcher::new(ServiceExtensionConfig {
        service_id,
        contract_address: contract,
        endpoint: endpoint.clone(),
        interval: Duration::from_secs(1),
    });
    let summary = watcher.tick().await.expect("watcher tick failed");
    assert_eq!(summary.running, 1);
    assert_eq!(
        summary.extended, 1,
        "watcher must raise the lifetime after the on-chain extension"
    );
    assert_eq!(
        store
            .get("ext-target")?
            .expect("target record")
            .max_lifetime_seconds,
        extended_deadline - record_created,
        "lifetime must now end exactly at the extended paid window"
    );
    assert_eq!(
        store
            .get("ext-other-service")?
            .expect("decoy record")
            .max_lifetime_seconds,
        DECOY_LIFETIME,
        "sandboxes of other services must not be touched"
    );

    // ── Real reaper lifecycle on the wall clock ──
    sleep_until(original_deadline + 5).await;
    reaper_tick().await;
    let target: Option<SandboxRecord> = store.get("ext-target")?;
    assert!(
        target.is_some(),
        "sandbox must survive past its original deadline after the extension"
    );
    let decoy: Option<SandboxRecord> = store.get("ext-other-service")?;
    assert!(decoy.is_some());

    sleep_until(extended_deadline + 15).await;
    reaper_tick().await;
    let target: Option<SandboxRecord> = store.get("ext-target")?;
    assert!(
        target.is_none(),
        "sandbox must be reaped once the extended deadline passes"
    );
    let decoy: Option<SandboxRecord> = store.get("ext-other-service")?;
    assert!(
        decoy.is_some(),
        "other services' sandboxes must survive"
    );
    Ok(())
}
