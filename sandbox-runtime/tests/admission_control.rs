//! Admission-control end-to-end: sandbox count cap on the TEE path,
//! per-sandbox resource maxima, and the host memory budget — all exercised
//! through the real `create_sidecar` entry point with the mock TEE backend
//! (no Docker required).
//!
//! Lives in its own test binary (own process) because `SidecarRuntimeConfig`
//! is a process-global `OnceCell` snapshot of the environment: the knobs
//! below must be set before the first `load()` and would leak into every
//! other in-crate test otherwise. The single #[tokio::test] keeps the store
//! mutations sequential.

#![cfg(feature = "test-utils")]

use sandbox_runtime::error::SandboxError;
use sandbox_runtime::runtime::{CreateSandboxParams, SandboxState, create_sidecar, sandboxes};
use sandbox_runtime::tee::mock::MockTeeBackend;
use sandbox_runtime::tee::{TeeConfig, TeeType};

fn tee_params(name: &str, cpu_cores: u64, memory_mb: u64) -> CreateSandboxParams {
    CreateSandboxParams {
        name: name.into(),
        image: "test:latest".into(),
        tee_config: Some(TeeConfig {
            required: true,
            tee_type: TeeType::Tdx,
            attestation_nonce: None,
        }),
        owner: "0xadmission".into(),
        cpu_cores,
        memory_mb,
        ..Default::default()
    }
}

fn expect_unavailable(err: SandboxError, needle: &str) {
    match err {
        SandboxError::Unavailable(msg) => {
            assert!(msg.contains(needle), "Unavailable message missing {needle:?}: {msg}");
        }
        other => panic!("expected SandboxError::Unavailable({needle:?}), got {other:?}"),
    }
}

#[tokio::test]
async fn admission_control_end_to_end() {
    let _guard = sandbox_runtime::TEST_ENV_GUARD
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let state_dir = tempfile::tempdir().unwrap();
    // SAFETY: single test in this binary; env set before the first
    // SidecarRuntimeConfig::load() under the process-wide env guard.
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
        std::env::set_var("SIDECAR_IMAGE", "test:latest");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("SANDBOX_MAX_COUNT", "3");
        std::env::set_var("SANDBOX_MAX_CPU_CORES", "4");
        std::env::set_var("SANDBOX_MAX_MEMORY_MB", "2048");
        std::env::set_var("SANDBOX_MAX_DISK_GB", "50");
        std::env::set_var("SANDBOX_HOST_MEMORY_BUDGET_MB", "4096");
    }
    let mock = MockTeeBackend::new(TeeType::Tdx);

    // Within every cap → admitted, requested values stored untouched.
    let (first, _) = create_sidecar(&tee_params("a", 2, 1024), Some(&mock))
        .await
        .expect("in-range request must be admitted");
    assert_eq!(first.memory_mb, 1024);
    assert_eq!(first.cpu_cores, 2);

    // Unlimited (0) memory with a max set → clamped to the max, not unlimited.
    let (second, _) = create_sidecar(&tee_params("b", 2, 0), Some(&mock))
        .await
        .expect("unlimited request must clamp, not reject");
    assert_eq!(
        second.memory_mb, 2048,
        "memory_mb=0 must clamp to SANDBOX_MAX_MEMORY_MB"
    );

    // Over the per-sandbox memory max → Unavailable.
    let err = create_sidecar(&tee_params("c", 2, 4096), Some(&mock))
        .await
        .unwrap_err();
    expect_unavailable(err, "memory_mb");

    // Over the per-sandbox CPU max → Unavailable.
    let err = create_sidecar(&tee_params("d", 8, 1024), Some(&mock))
        .await
        .unwrap_err();
    expect_unavailable(err, "cpu_cores");

    // Over the per-sandbox disk max → Unavailable.
    let mut disk_heavy = tee_params("e", 2, 1024);
    disk_heavy.disk_gb = 500;
    let err = create_sidecar(&disk_heavy, Some(&mock)).await.unwrap_err();
    expect_unavailable(err, "disk_gb");

    // Host memory budget: 1024 + 2048 running; +2048 = 5120 > 4096 → Unavailable.
    let err = create_sidecar(&tee_params("f", 2, 2048), Some(&mock))
        .await
        .unwrap_err();
    expect_unavailable(err, "memory budget");

    // Exactly at budget (3072 + 1024 = 4096) → admitted.
    let (third, _) = create_sidecar(&tee_params("g", 2, 1024), Some(&mock))
        .await
        .expect("request exactly at the budget must be admitted");

    // Stopping a sandbox frees its budget share but keeps its store slot, so
    // the next create passes the budget and hits the COUNT cap — proving the
    // TEE path enforces SANDBOX_MAX_COUNT and rejects with the 503 class.
    assert!(
        sandboxes()
            .unwrap()
            .update(&third.id, |record| record.state = SandboxState::Stopped)
            .unwrap()
    );
    let err = create_sidecar(&tee_params("h", 2, 1024), Some(&mock))
        .await
        .unwrap_err();
    expect_unavailable(err, "Sandbox limit reached");
}
