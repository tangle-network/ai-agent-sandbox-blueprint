//! The warm pool's standing memory footprint is reserved against
//! `SANDBOX_HOST_MEMORY_BUDGET_MB` at admission, so an enabled pool cannot
//! silently over-commit host RAM. Pool inventory (templates + pre-restored
//! entries) never enters the sandbox store, so without this reservation the
//! host budget is blind to it.
//!
//! Named bug it catches: dropping the `reserved_host_memory_mb()` term from
//! `enforce_host_memory_budget`. With it gone, a create that only fits because
//! the pool's footprint is ignored is wrongly admitted, over-committing the
//! host — the third create below succeeds when it must not.
//!
//! Own test binary (own process): `SidecarRuntimeConfig` is a process-global
//! snapshot of the environment (same convention as `firecracker_warm_admission.rs`).

#![cfg(feature = "test-utils")]

use sandbox_runtime::error::SandboxError;
use sandbox_runtime::runtime::{CreateSandboxParams, create_sidecar};
use sandbox_runtime::tee::mock::MockTeeBackend;
use sandbox_runtime::tee::{TeeConfig, TeeType};

fn tee_create(name: &str) -> CreateSandboxParams {
    CreateSandboxParams {
        name: name.into(),
        image: "test:latest".into(),
        tee_config: Some(TeeConfig {
            required: true,
            tee_type: TeeType::Tdx,
            attestation_nonce: None,
        }),
        owner: "0xwarm".into(),
        cpu_cores: 1,
        memory_mb: 1024,
        ..Default::default()
    }
}

/// Budget 4096 MB, warm pool = 1 generation × 2 × 1024 MiB = 2048 MB reserved.
/// Two 1024 MB sandboxes fit exactly at the budget with the reservation
/// counted; a third must be rejected. Without the reservation the third would
/// only sum to 3072 ≤ 4096 and be wrongly admitted.
#[tokio::test]
async fn warm_pool_reservation_counts_against_host_budget() {
    let state_dir = tempfile::tempdir().unwrap();
    {
        let _guard = sandbox_runtime::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: env set under the process-wide guard, before the first
        // SidecarRuntimeConfig::load() in this process.
        unsafe {
            std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
            std::env::set_var("SIDECAR_IMAGE", "test:latest");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            std::env::set_var("SANDBOX_HOST_MEMORY_BUDGET_MB", "4096");
            std::env::set_var("SANDBOX_MAX_MEMORY_MB", "2048");
            // Count is deliberately not the binding limit — the budget is.
            std::env::set_var("SANDBOX_MAX_COUNT", "16");
            std::env::set_var("SANDBOX_FC_WARM_POOL_SIZE", "1");
            std::env::set_var("MICROVM_FIRECRACKER_MEM_MIB", "1024");
            // The mock TEE path never dispatches to Firecracker; point the FC
            // artifacts nowhere so any accidental boot fails loudly.
            std::env::set_var("MICROVM_FIRECRACKER_BIN", "/nonexistent/firecracker");
            std::env::set_var("MICROVM_FIRECRACKER_KERNEL", "/nonexistent/vmlinux");
            std::env::set_var("MICROVM_FIRECRACKER_ROOTFS", "/nonexistent/rootfs");
        }
    }

    let mock = MockTeeBackend::new(TeeType::Tdx);

    // 2048 reserved + 1024 incoming = 3072 ≤ 4096 → admitted.
    create_sidecar(&tee_create("sb1"), Some(&mock))
        .await
        .expect("create #1 fits under budget with the pool reservation");

    // 2048 reserved + 1024 running + 1024 incoming = 4096 == 4096 → admitted.
    create_sidecar(&tee_create("sb2"), Some(&mock))
        .await
        .expect("create #2 sits exactly at the budget");

    // 2048 reserved + 2048 running + 1024 incoming = 5120 > 4096 → rejected.
    let err = create_sidecar(&tee_create("sb3"), Some(&mock))
        .await
        .expect_err("create #3 must be rejected once the pool reservation is counted");
    match err {
        SandboxError::Unavailable(msg) => assert!(
            msg.contains("memory budget") && msg.contains("warm-pool reserved"),
            "expected a budget rejection naming the warm-pool reservation, got: {msg}"
        ),
        other => panic!("expected SandboxError::Unavailable, got {other:?}"),
    }
}
