//! TEE config decoding, type mapping, deploy params, and persistence roundtrip tests.
//!
//! All tests run without Docker or env-var gates — pure unit tests for
//! TEE-specific config paths.

use std::sync::Once;

use ai_agent_tee_instance_blueprint_lib::*;
use sandbox_runtime::tee::TeeDeployParams;

static INIT: Once = Once::new();
static INSTANCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Helper to build a ProvisionRequest with only TEE-relevant fields set.
fn make_provision_request(name: &str, tee_required: bool, tee_type: u8) -> ProvisionRequest {
    ProvisionRequest {
        name: name.into(),
        image: "nginx:alpine".into(),
        stack: String::new(),
        agent_identifier: String::new(),
        env_json: String::new(),
        metadata_json: String::new(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 0,
        idle_timeout_seconds: 0,
        cpu_cores: 0,
        memory_mb: 0,
        disk_gb: 0,
        tee_required,
        tee_type,
        attestation_nonce: String::new(),
    }
}

fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("tee-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        // SAFETY: tests run single-threaded during init; no concurrent env reads.
        unsafe {
            std::env::set_var("BLUEPRINT_STATE_DIR", dir.to_str().unwrap());
            std::env::set_var("SIDECAR_IMAGE", "nginx:alpine");
            std::env::set_var("SIDECAR_PULL_IMAGE", "false");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            std::env::set_var("REQUEST_TIMEOUT_SECS", "10");
        }
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// DECODE PROVISION CONFIG — TEE fields
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn decode_provision_config_tee_required_tdx() {
    use ai_agent_tee_instance_blueprint_lib::auto_provision::decode_provision_config;
    use blueprint_sdk::alloy::sol_types::SolValue;

    let req = ProvisionRequest {
        name: "tee-tdx".into(),
        image: "nginx:alpine".into(),
        stack: String::new(),
        agent_identifier: String::new(),
        env_json: String::new(),
        metadata_json: String::new(),
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 300,
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 50,
        tee_required: true,
        tee_type: 1,
        attestation_nonce: String::new(), // Tdx
    };

    let encoded = req.abi_encode_params();
    let decoded = decode_provision_config(&encoded).expect("decode should succeed");

    assert_eq!(decoded.name, "tee-tdx");
    assert!(decoded.tee_required);
    assert_eq!(decoded.tee_type, 1);

    // Verify conversion to CreateSandboxParams preserves TEE config.
    let params = CreateSandboxParams::from(&decoded);
    let tee_config = params.tee_config.expect("tee_config should be Some");
    assert!(tee_config.required);
    assert_eq!(tee_config.tee_type, TeeType::Tdx);
}

#[test]
fn decode_provision_config_tee_types_nitro_sev() {
    use ai_agent_tee_instance_blueprint_lib::auto_provision::decode_provision_config;
    use blueprint_sdk::alloy::sol_types::SolValue;

    // Test Nitro (tee_type=2)
    let req_nitro = make_provision_request("tee-nitro", true, 2);
    let decoded_nitro =
        decode_provision_config(&req_nitro.abi_encode_params()).expect("nitro decode");
    let params_nitro = CreateSandboxParams::from(&decoded_nitro);
    let cfg_nitro = params_nitro.tee_config.expect("nitro tee_config");
    assert_eq!(cfg_nitro.tee_type, TeeType::Nitro);

    // Test Sev (tee_type=3)
    let req_sev = make_provision_request("tee-sev", true, 3);
    let decoded_sev = decode_provision_config(&req_sev.abi_encode_params()).expect("sev decode");
    let params_sev = CreateSandboxParams::from(&decoded_sev);
    let cfg_sev = params_sev.tee_config.expect("sev tee_config");
    assert_eq!(cfg_sev.tee_type, TeeType::Sev);
}

#[test]
fn decode_provision_config_unknown_tee_type_maps_to_none() {
    use ai_agent_tee_instance_blueprint_lib::auto_provision::decode_provision_config;
    use blueprint_sdk::alloy::sol_types::SolValue;

    let req = make_provision_request("tee-unknown", true, 99);

    let decoded = decode_provision_config(&req.abi_encode_params()).expect("decode");
    let params = CreateSandboxParams::from(&decoded);
    let cfg = params
        .tee_config
        .expect("tee_config should be Some when tee_required=true");
    assert_eq!(cfg.tee_type, TeeType::None);
}

#[test]
#[allow(clippy::type_complexity)]
fn tee_config_conversion_all_variants() {
    // Table-driven: (tee_required, tee_type) → expected tee_config
    let cases: Vec<(bool, u8, Option<(bool, TeeType)>)> = vec![
        (false, 0, None),                         // not required, type none
        (true, 0, Some((true, TeeType::None))),   // required, type 0 → None
        (true, 1, Some((true, TeeType::Tdx))),    // required, Tdx
        (true, 2, Some((true, TeeType::Nitro))),  // required, Nitro
        (true, 3, Some((true, TeeType::Sev))),    // required, Sev
        (true, 255, Some((true, TeeType::None))), // required, unknown → None
    ];

    for (tee_required, tee_type, expected) in cases {
        let req = make_provision_request("test", tee_required, tee_type);

        let params = CreateSandboxParams::from(&req);
        match expected {
            None => {
                assert!(
                    params.tee_config.is_none(),
                    "tee_required={tee_required}, tee_type={tee_type}: expected None"
                );
            }
            Some((req_flag, expected_type)) => {
                let cfg = params.tee_config.as_ref().unwrap_or_else(|| {
                    panic!("tee_required={tee_required}, tee_type={tee_type}: expected Some")
                });
                assert_eq!(cfg.required, req_flag);
                assert_eq!(cfg.tee_type, expected_type);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEE DEPLOY PARAMS — field mapping
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tee_deploy_params_full_field_mapping() {
    let params = CreateSandboxParams {
        name: "dp-test".into(),
        image: "my-image:latest".into(),
        env_json: r#"{"MY_VAR":"hello","ANOTHER":"world"}"#.into(),
        ssh_enabled: true,
        cpu_cores: 4,
        memory_mb: 8192,
        disk_gb: 100,
        port_mappings: vec![3000, 9090],
        tee_config: Some(TeeConfig {
            required: true,
            tee_type: TeeType::Tdx,
            attestation_nonce: None,
        }),
        ..Default::default()
    };

    let deploy = TeeDeployParams::from_sandbox_params("sb-123", &params, 8080, 22, "my-token");

    assert_eq!(deploy.sandbox_id, "sb-123");
    assert_eq!(deploy.image, "my-image:latest");
    assert_eq!(deploy.cpu_cores, 4);
    assert_eq!(deploy.memory_mb, 8192);
    assert_eq!(deploy.disk_gb, 100);
    assert_eq!(deploy.http_port, 8080);
    assert_eq!(deploy.ssh_port, Some(22));
    assert_eq!(deploy.sidecar_token, "my-token");
    assert_eq!(deploy.extra_ports, vec![3000, 9090]);

    // Env vars should include SIDECAR_PORT + SIDECAR_AUTH_TOKEN + user vars.
    let env_map: std::collections::HashMap<&str, &str> = deploy
        .env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(env_map.get("SIDECAR_PORT"), Some(&"8080"));
    assert_eq!(env_map.get("SIDECAR_AUTH_TOKEN"), Some(&"my-token"));
    assert_eq!(env_map.get("MY_VAR"), Some(&"hello"));
    assert_eq!(env_map.get("ANOTHER"), Some(&"world"));
}

#[test]
fn tee_deploy_params_ssh_disabled_omits_port() {
    let params = CreateSandboxParams {
        name: "no-ssh".into(),
        image: "nginx:alpine".into(),
        ssh_enabled: false,
        cpu_cores: 1,
        memory_mb: 512,
        disk_gb: 10,
        ..Default::default()
    };

    let deploy = TeeDeployParams::from_sandbox_params("sb-nossh", &params, 8080, 22, "tok");

    assert_eq!(deploy.ssh_port, None);
}

// ═══════════════════════════════════════════════════════════════════════════
// PERSISTENCE ROUNDTRIP — TEE fields survive seal/unseal
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tee_fields_persistence_roundtrip() {
    init();
    let _guard = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    clear_instance_sandbox().unwrap();

    let record = SandboxRecord {
        id: "tee-roundtrip".into(),
        container_id: "mock-deploy-rt".into(),
        sidecar_url: "http://localhost:9999".into(),
        sidecar_port: 8080,
        ssh_port: Some(2222),
        token: "roundtrip-tok".into(),
        created_at: 1000,
        cpu_cores: 2,
        memory_mb: 4096,
        state: SandboxState::Running,
        idle_timeout_seconds: 300,
        max_lifetime_seconds: 3600,
        last_activity_at: 1000,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: "nginx:alpine".into(),
        base_env_json: String::new(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: Some("deploy-rt-001".into()),
        tee_metadata_json: Some(r#"{"backend":"mock","region":"us-east"}"#.into()),
        tee_attestation_json: Some(
            r#"{"tee_type":"Tdx","evidence":[222,173],"measurement":[190,239],"timestamp":1700000000}"#.into(),
        ),
        name: "roundtrip-test".into(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 50,
        stack: String::new(),
        owner: "0xdeadbeef00000000000000000000000000000001".into(),
        service_id: None,
        tee_config: Some(TeeConfig {
            required: true,
            tee_type: TeeType::Tdx,
            attestation_nonce: None,
        }),
        extra_ports: std::collections::HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
    };

    set_instance_sandbox(record).unwrap();

    let loaded = get_instance_sandbox()
        .unwrap()
        .expect("record should exist after set");

    // TEE-specific fields survived the seal/unseal roundtrip.
    assert_eq!(loaded.tee_deployment_id.as_deref(), Some("deploy-rt-001"));
    assert!(
        loaded.tee_metadata_json.as_ref().unwrap().contains("mock"),
        "tee_metadata_json should survive roundtrip"
    );
    assert!(
        loaded
            .tee_attestation_json
            .as_ref()
            .unwrap()
            .contains("Tdx"),
        "tee_attestation_json should survive roundtrip"
    );
    assert!(
        loaded
            .tee_attestation_json
            .as_ref()
            .unwrap()
            .contains("222"),
        "attestation evidence bytes should survive roundtrip"
    );

    let cfg = loaded
        .tee_config
        .expect("tee_config should survive roundtrip");
    assert!(cfg.required);
    assert_eq!(cfg.tee_type, TeeType::Tdx);

    // Cleanup.
    clear_instance_sandbox().unwrap();
}
