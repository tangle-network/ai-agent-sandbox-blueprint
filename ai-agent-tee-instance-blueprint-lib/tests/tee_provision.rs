//! TEE provision/deprovision lifecycle tests using MockTeeBackend.
//!
//! All tests run without Docker or env-var gates — `MockTeeBackend` replaces
//! all real TEE infrastructure.
//!
//! Run with:
//! ```bash
//! cargo test -p ai-agent-tee-instance-blueprint-lib --test tee_provision
//! ```

use std::sync::Once;
use std::sync::atomic::Ordering;

use ai_agent_tee_instance_blueprint_lib::*;
use sandbox_runtime::tee::mock::MockTeeBackend;
use sandbox_runtime::tee::AttestationReport;

static INIT: Once = Once::new();
static INSTANCE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn init() {
    INIT.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("tee-provision-test-{}", std::process::id()));
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

fn tee_provision_request() -> ProvisionRequest {
    ProvisionRequest {
        name: "tee-test".into(),
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
        cpu_cores: 1,
        memory_mb: 512,
        disk_gb: 10,
        sidecar_token: String::new(),
        tee_required: true,
        tee_type: 1, // Tdx
    }
}

/// Clean both instance store and runtime sandboxes store for the given ID.
fn cleanup(sandbox_id: Option<&str>) {
    let _ = clear_instance_sandbox();
    if let Some(id) = sandbox_id {
        if let Ok(store) = runtime::sandboxes() {
            let _ = store.remove(id);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROVISION LIFECYCLE
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provision_core_tee_full_lifecycle() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::new(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000001";

    let (output, record) = provision_core(&req, Some(&mock), owner)
        .await
        .expect("provision_core should succeed with MockTeeBackend");

    // Store the record (as the real caller would).
    set_instance_sandbox(record.clone()).unwrap();

    // Verify ProvisionOutput has real attestation (not pending).
    assert!(
        !output.tee_attestation_json.is_empty(),
        "attestation JSON should be populated"
    );
    assert!(
        output.tee_attestation_json.contains("222"),
        "attestation should contain evidence byte 0xDE (222): {}",
        output.tee_attestation_json
    );
    assert!(
        output.tee_attestation_json.contains("Tdx"),
        "attestation should contain TEE type: {}",
        output.tee_attestation_json
    );

    // Verify TEE public key was derived.
    assert!(
        !output.tee_public_key_json.is_empty(),
        "tee_public_key_json should be populated"
    );
    assert!(
        output.tee_public_key_json.contains("x25519-hkdf-sha256"),
        "public key should contain algorithm: {}",
        output.tee_public_key_json
    );

    // Verify SandboxRecord has TEE fields.
    assert!(record.tee_deployment_id.is_some());
    assert!(record.tee_metadata_json.is_some());
    assert!(record.tee_config.is_some());

    // Verify mock call counts.
    assert_eq!(mock.deploy_count.load(Ordering::Relaxed), 1);
    assert_eq!(mock.derive_pk_count.load(Ordering::Relaxed), 1);

    cleanup(Some(&record.id));
}

#[tokio::test]
async fn provision_core_tee_no_backend_rejects() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000002";

    let result = provision_core(&req, None, owner).await;

    assert!(result.is_err(), "provision should fail with tee=None when tee_required=true");
    let err = match result { Err(e) => e, Ok(_) => panic!("expected error") };
    assert!(
        err.contains("no TEE backend configured"),
        "error should mention missing backend: {err}"
    );

    cleanup(None);
}

#[tokio::test]
async fn provision_core_tee_attestation_is_real_not_pending() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::new(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000003";

    let (output, record) = provision_core(&req, Some(&mock), owner)
        .await
        .expect("provision should succeed");

    // Deserialize and verify it's a real AttestationReport, not the pending placeholder.
    let attestation: AttestationReport =
        serde_json::from_str(&output.tee_attestation_json).expect("should deserialize as AttestationReport");

    assert_eq!(attestation.evidence, vec![0xDE, 0xAD]);
    assert_eq!(attestation.measurement, vec![0xBE, 0xEF]);
    assert_eq!(attestation.timestamp, 1_700_000_000);

    // Confirm it does NOT contain the pending placeholder.
    assert!(
        !output.tee_attestation_json.contains("pending"),
        "should not contain pending placeholder: {}",
        output.tee_attestation_json
    );

    cleanup(Some(&record.id));
}

#[tokio::test]
async fn provision_core_tee_pk_failure_non_fatal() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::new(TeeType::Tdx);
    // Disable sealed secrets support so derive_public_key fails.
    mock.support_sealed_secrets.store(false, Ordering::Relaxed);

    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000004";

    let (output, record) = provision_core(&req, Some(&mock), owner)
        .await
        .expect("provision should succeed even when PK derivation fails");

    // Public key should be empty (graceful degradation).
    assert!(
        output.tee_public_key_json.is_empty(),
        "tee_public_key_json should be empty when derivation fails: {}",
        output.tee_public_key_json
    );

    // Attestation should still be populated.
    assert!(
        !output.tee_attestation_json.is_empty(),
        "attestation should still be present"
    );

    // derive_public_key was attempted.
    assert_eq!(mock.derive_pk_count.load(Ordering::Relaxed), 1);

    cleanup(Some(&record.id));
}

#[tokio::test]
async fn provision_core_tee_deploy_failure_propagates() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::failing(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000005";

    let result = provision_core(&req, Some(&mock), owner).await;

    assert!(result.is_err(), "provision should fail when deploy fails");
    let err = match result { Err(e) => e, Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Mock deploy failure"),
        "error should propagate mock failure message: {err}"
    );

    // Deploy was attempted.
    assert_eq!(mock.deploy_count.load(Ordering::Relaxed), 1);

    // Instance should NOT be stored.
    let stored = get_instance_sandbox().unwrap();
    assert!(stored.is_none(), "no record should be stored on deploy failure");

    cleanup(None);
}

#[tokio::test]
async fn provision_core_tee_already_provisioned_rejects() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::new(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000006";

    // First provision succeeds.
    let (_, record) = provision_core(&req, Some(&mock), owner)
        .await
        .expect("first provision should succeed");
    set_instance_sandbox(record.clone()).unwrap();

    // Second provision should fail.
    let result = provision_core(&req, Some(&mock), owner).await;
    assert!(result.is_err());
    let err = match result { Err(e) => e, Ok(_) => panic!("expected error") };
    assert!(
        err.contains("already provisioned"),
        "should reject duplicate provision: {err}"
    );

    // Only one deploy call.
    assert_eq!(mock.deploy_count.load(Ordering::Relaxed), 1);

    cleanup(Some(&record.id));
}

// ═══════════════════════════════════════════════════════════════════════════
// DEPROVISION
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn deprovision_core_calls_tee_destroy() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    let mock = MockTeeBackend::new(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000007";

    // Provision first.
    let (_, record) = provision_core(&req, Some(&mock), owner)
        .await
        .expect("provision should succeed");
    set_instance_sandbox(record.clone()).unwrap();

    // Deprovision.
    let (response, sandbox_id) = deprovision_core(Some(&mock))
        .await
        .expect("deprovision should succeed");

    assert_eq!(sandbox_id, record.id);
    assert!(response.json.contains("deprovisioned"));

    // TEE destroy was called.
    assert_eq!(mock.destroy_count.load(Ordering::Relaxed), 1);

    // Instance store should be cleared.
    let stored = get_instance_sandbox().unwrap();
    assert!(stored.is_none(), "instance should be cleared after deprovision");

    cleanup(None);
}

#[tokio::test]
async fn deprovision_core_tee_destroy_failure_propagates() {
    init();
    let _guard = INSTANCE_LOCK.lock().await;
    cleanup(None);

    // Use a working mock for provisioning.
    let working_mock = MockTeeBackend::new(TeeType::Tdx);
    let req = tee_provision_request();
    let owner = "0xdeadbeef00000000000000000000000000000008";

    let (_, record) = provision_core(&req, Some(&working_mock), owner)
        .await
        .expect("provision should succeed");
    set_instance_sandbox(record.clone()).unwrap();

    // Use a failing mock for deprovisioning.
    let failing_mock = MockTeeBackend::failing(TeeType::Tdx);

    let result = deprovision_core(Some(&failing_mock)).await;
    assert!(result.is_err(), "deprovision should fail when destroy fails");
    let err = match result { Err(e) => e, Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Mock destroy failure") || err.contains("Mock"),
        "error should propagate: {err}"
    );

    // Instance record should still exist (cleanup not performed on failure).
    let stored = get_instance_sandbox().unwrap();
    assert!(
        stored.is_some(),
        "record should still exist after failed deprovision"
    );

    cleanup(Some(&record.id));
}
