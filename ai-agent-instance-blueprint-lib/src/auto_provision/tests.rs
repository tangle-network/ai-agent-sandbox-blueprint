use super::*;
use std::collections::HashMap;

fn test_record(service_id: Option<u64>, owner: &str) -> crate::SandboxRecord {
    crate::SandboxRecord {
        id: "sandbox-test-1".to_string(),
        container_id: "ctr-test-1".to_string(),
        sidecar_url: "http://127.0.0.1:9202".to_string(),
        sidecar_port: 9202,
        ssh_port: None,
        token: "token".to_string(),
        created_at: 1,
        cpu_cores: 2,
        memory_mb: 2048,
        state: crate::SandboxState::Running,
        idle_timeout_seconds: 0,
        max_lifetime_seconds: 0,
        last_activity_at: 1,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "ghcr.io/tangle-network/blueprint-sidecar:all-harness".to_string(),
        base_env_json: "{}".to_string(),
        user_env_json: "{}".to_string(),
        snapshot_destination: None,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: "instance".to_string(),
        agent_identifier: "test-agent".to_string(),
        metadata_json: "{}".to_string(),
        disk_gb: 20,
        stack: "default".to_string(),
        owner: owner.to_string(),
        service_id,
        tee_config: None,
        extra_ports: HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: String::new(),
    }
}

#[test]
fn config_from_env_returns_none_without_bsm() {
    // BSM_ADDRESS not set → None
    unsafe { std::env::remove_var("BSM_ADDRESS") };
    assert!(AutoProvisionConfig::from_env(1).is_none());
}

#[test]
fn bind_service_id_sets_binding() {
    let record = bind_service_id(test_record(None, "0xabc"), 7);
    assert_eq!(record.service_id, Some(7));
}

#[test]
fn reuse_check_accepts_matching_bound_service() {
    let record = test_record(Some(7), "0xabc");
    assert!(should_reuse_existing_record(&record, 7, None));
}

#[test]
fn reuse_check_accepts_legacy_record_for_same_owner() {
    let record = test_record(None, "0xabc");
    assert!(should_reuse_existing_record(&record, 7, Some("0xAbC")));
}

#[test]
fn reuse_check_rejects_legacy_record_for_different_owner() {
    let record = test_record(None, "0xabc");
    assert!(!should_reuse_existing_record(&record, 7, Some("0xdef")));
}

#[test]
fn decode_provision_config_roundtrip() {
    use blueprint_sdk::alloy::sol_types::SolValue;

    let request = ProvisionRequest {
        name: "test-sandbox".to_string(),
        image: "ghcr.io/tangle-network/blueprint-sidecar:all-harness".to_string(),
        stack: "default".to_string(),
        agent_identifier: "test-agent".to_string(),
        env_json: "{}".to_string(),
        metadata_json: "{}".to_string(),
        ssh_enabled: true,
        ssh_public_key: "ssh-ed25519 AAAA test".to_string(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 900,
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 20,
        tee_required: false,
        tee_type: 0,
        attestation_nonce: String::new(),
        capabilities_json: String::new(),
    };

    // On-chain config is stored as params encoding (flat tuple, no outer offset),
    // matching `cast abi-encode` / `abi.encode(field1, field2, ...)`.
    let encoded = request.abi_encode_params();
    let decoded = decode_provision_config(&encoded).unwrap();

    assert_eq!(decoded.name, "test-sandbox");
    assert_eq!(
        decoded.image,
        "ghcr.io/tangle-network/blueprint-sidecar:all-harness"
    );
    assert_eq!(decoded.cpu_cores, 2);
    assert_eq!(decoded.memory_mb, 4096);
    assert!(decoded.ssh_enabled);
}

#[test]
fn decode_provision_config_tuple_encoding() {
    use blueprint_sdk::alloy::sol_types::SolValue;

    let request = ProvisionRequest {
        name: "tuple-sandbox".to_string(),
        image: "ghcr.io/tangle-network/blueprint-sidecar:all-harness".to_string(),
        stack: "default".to_string(),
        agent_identifier: "test-agent".to_string(),
        env_json: r#"{"KEY":"VALUE"}"#.to_string(),
        metadata_json: "{}".to_string(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: true,
        max_lifetime_seconds: 7200,
        idle_timeout_seconds: 1800,
        cpu_cores: 4,
        memory_mb: 8192,
        disk_gb: 40,
        tee_required: true,
        tee_type: 1,
        attestation_nonce: String::new(),
        capabilities_json: String::new(),
    };

    // abi_encode() produces tuple encoding (with outer offset prefix).
    let encoded = request.abi_encode();
    let decoded = decode_provision_config(&encoded).unwrap();

    assert_eq!(decoded.name, "tuple-sandbox");
    assert_eq!(decoded.cpu_cores, 4);
    assert_eq!(decoded.memory_mb, 8192);
    assert!(decoded.tee_required);
    assert_eq!(decoded.tee_type, 1);
}

#[test]
fn decode_provision_config_preserves_attestation_nonce() {
    use blueprint_sdk::alloy::sol_types::SolValue;

    let nonce = "11".repeat(32);
    let request = ProvisionRequest {
        name: "nonce-sandbox".to_string(),
        image: "ghcr.io/tangle-network/blueprint-sidecar:all-harness".to_string(),
        stack: "default".to_string(),
        agent_identifier: "test-agent".to_string(),
        env_json: "{}".to_string(),
        metadata_json: "{}".to_string(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 900,
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 20,
        tee_required: true,
        tee_type: 1,
        attestation_nonce: nonce.clone(),
        capabilities_json: String::new(),
    };

    let encoded = request.abi_encode_params();
    let decoded = decode_provision_config(&encoded).unwrap();

    assert_eq!(decoded.name, "nonce-sandbox");
    assert!(decoded.tee_required);
    assert_eq!(decoded.tee_type, 1);
    assert_eq!(decoded.attestation_nonce, nonce);
}

#[test]
fn decode_provision_config_legacy_shape_without_using_sidecar_token() {
    use blueprint_sdk::alloy::sol_types::SolValue;

    let request = LegacyProvisionRequest {
        name: "legacy-sandbox".to_string(),
        image: "ghcr.io/tangle-network/blueprint-sidecar:all-harness".to_string(),
        stack: "default".to_string(),
        agent_identifier: "test-agent".to_string(),
        env_json: "{}".to_string(),
        metadata_json: "{}".to_string(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 900,
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 20,
        sidecar_token: "legacy-token".to_string(),
        tee_required: false,
        tee_type: 0,
    };

    let encoded = request.abi_encode_params();
    let decoded = decode_provision_config(&encoded).unwrap();

    assert_eq!(decoded.name, "legacy-sandbox");
    assert_eq!(
        decoded.image,
        "ghcr.io/tangle-network/blueprint-sidecar:all-harness"
    );
    assert_eq!(decoded.cpu_cores, 2);
    assert_eq!(decoded.memory_mb, 4096);
    assert!(!decoded.tee_required);
}

#[test]
fn decode_provision_config_malformed_bytes_rejected() {
    let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
    let result = decode_provision_config(&garbage);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.contains("Failed to decode"), "got: {err}");
}
