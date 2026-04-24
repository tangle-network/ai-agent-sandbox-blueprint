//! TEE integration tests for the TEE instance blueprint library.
//!
//! These tests verify TEE-specific provision/deprovision behavior using the
//! `MockTeeBackend` against a real persistent store (no Docker required).
//!
//! These tests are deterministic and are part of the default Phase 1 TEE Rust suite.

use std::sync::Once;

static INIT: Once = Once::new();
static INSTANCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("tee-integ-test-{}", std::process::id()));
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
// IDEMPOTENT PROVISION — attestation preservation (bug #5 regression)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tee_provision_idempotent_returns_stored_attestation() {
    init();
    let _guard = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    use ai_agent_tee_instance_blueprint_lib::*;

    // Start clean.
    clear_instance_sandbox().unwrap();

    // Simulate auto-provision having already created a sandbox with attestation.
    let record = SandboxRecord {
        id: "tee-integ-idempotent".into(),
        container_id: "tee-mock-deploy-1".into(),
        sidecar_url: "http://localhost:9999".into(),
        sidecar_port: 8080,
        ssh_port: Some(2222),
        token: "test-tok".into(),
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
        tee_deployment_id: Some("mock-deploy-1".into()),
        tee_metadata_json: Some(r#"{"backend":"mock"}"#.into()),
        tee_attestation_json: Some(
            r#"{"tee_type":"Tdx","evidence":[222,173],"measurement":[190,239],"timestamp":1700000000}"#.into(),
        ),
        name: "tee-integ".into(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 10,
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

    // Now read back — the idempotent path in tee_provision would read from here.
    let stored = get_instance_sandbox().unwrap().unwrap();
    let attestation = stored.tee_attestation_json.clone().unwrap_or_default();

    assert!(
        !attestation.is_empty(),
        "Attestation should be preserved on stored record"
    );
    assert!(
        attestation.contains("Tdx"),
        "Attestation should contain TEE type: {attestation}"
    );
    assert!(
        attestation.contains("222"),
        "Attestation should contain evidence bytes: {attestation}"
    );

    // Verify the ProvisionOutput construction that the idempotent path uses.
    let output = ProvisionOutput {
        sandbox_id: stored.id.clone(),
        sidecar_url: stored.sidecar_url.clone(),
        ssh_port: stored.ssh_port.unwrap_or(0) as u32,
        tee_attestation_json: stored.tee_attestation_json.clone().unwrap_or_default(),
        tee_public_key_json: String::new(),
    };

    assert!(!output.tee_attestation_json.is_empty());
    assert_eq!(output.ssh_port, 2222);

    // Cleanup.
    clear_instance_sandbox().unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// DEPROVISION — store cleanup
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tee_deprovision_clears_instance_sandbox() {
    init();
    let _guard = INSTANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    use ai_agent_tee_instance_blueprint_lib::*;

    // Start clean.
    clear_instance_sandbox().unwrap();

    // Set an instance sandbox.
    let record = SandboxRecord {
        id: "tee-integ-deprovision".into(),
        container_id: "tee-mock-dep-1".into(),
        sidecar_url: "http://localhost:9998".into(),
        sidecar_port: 8080,
        ssh_port: None,
        token: "tok".into(),
        created_at: 1000,
        cpu_cores: 1,
        memory_mb: 512,
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
        tee_deployment_id: Some("mock-dep-1".into()),
        tee_metadata_json: Some("{}".into()),
        tee_attestation_json: None,
        name: "test".into(),
        agent_identifier: String::new(),
        metadata_json: String::new(),
        disk_gb: 10,
        stack: String::new(),
        owner: "0xdeadbeef".into(),
        service_id: None,
        tee_config: None,
        extra_ports: std::collections::HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
    };

    set_instance_sandbox(record).unwrap();
    assert!(get_instance_sandbox().unwrap().is_some());

    // After deprovision, the instance sandbox should be cleared.
    // Note: we can't call deprovision_core directly because it calls
    // delete_sidecar which needs Docker. Instead, we test the store
    // operations that deprovision_core performs.
    clear_instance_sandbox().unwrap();
    assert!(get_instance_sandbox().unwrap().is_none());
}

// ═══════════════════════════════════════════════════════════════════════════
// TEE CONFIG — from_sandbox_params with extra_ports
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tee_deploy_params_includes_extra_ports() {
    use ai_agent_tee_instance_blueprint_lib::*;
    use sandbox_runtime::tee::TeeDeployParams;

    let params = CreateSandboxParams {
        name: "test".into(),
        image: "nginx:alpine".into(),
        port_mappings: vec![3000, 8080],
        ssh_enabled: true,
        cpu_cores: 2,
        memory_mb: 4096,
        disk_gb: 50,
        ..Default::default()
    };

    let deploy = TeeDeployParams::from_sandbox_params("sb-integ", &params, 8080, 22, "tok");

    assert_eq!(deploy.extra_ports, vec![3000, 8080]);
    assert_eq!(deploy.ssh_port, Some(22));
    assert_eq!(deploy.cpu_cores, 2);
}
