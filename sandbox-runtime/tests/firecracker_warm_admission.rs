//! Admission-order invariant for warm-pool serving: a create rejected by
//! admission control must NEVER touch the Firecracker warm pool — no claim,
//! no seeding, not even engine construction. The warm claim lives inside
//! `firecracker::create_and_start`, which the runtime layer only calls
//! after `admit_sandbox_resources` + `enforce_sandbox_count_limit` pass;
//! this test pins that ordering from the outside.
//!
//! Named bug it catches: moving the warm claim (or `ensure_seeding`) above
//! the admission gates, which would let warm-claimed sandboxes evade
//! `SANDBOX_MAX_COUNT` / the host memory budget — pool inventory is
//! intentionally outside those budgets, so the *claim* being admitted is
//! the entire accounting story.
//!
//! Own test binary (own process): `SidecarRuntimeConfig` is a process-global
//! snapshot of the environment (same convention as `admission_control.rs`).

#![cfg(feature = "test-utils")]

use sandbox_runtime::error::SandboxError;
use sandbox_runtime::runtime::{CreateSandboxParams, create_sidecar};
use sandbox_runtime::tee::mock::MockTeeBackend;
use sandbox_runtime::tee::{TeeConfig, TeeType};

fn expect_unavailable(err: SandboxError, needle: &str) {
    match err {
        SandboxError::Unavailable(msg) => {
            assert!(
                msg.contains(needle),
                "Unavailable message missing {needle:?}: {msg}"
            );
        }
        other => panic!("expected SandboxError::Unavailable({needle:?}), got {other:?}"),
    }
}

#[tokio::test]
async fn rejected_create_never_touches_warm_pool() {
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
            std::env::set_var("SANDBOX_MAX_COUNT", "1");
            // Warm pool nominally ON — the point of the test. Also point
            // the FC primitive at nonexistent artifacts so any accidental
            // seed/claim attempt fails loudly instead of booting VMs.
            std::env::set_var("SANDBOX_FC_WARM_POOL_SIZE", "2");
            std::env::set_var("MICROVM_FIRECRACKER_BIN", "/nonexistent/firecracker");
            std::env::set_var("MICROVM_FIRECRACKER_KERNEL", "/nonexistent/vmlinux");
            std::env::set_var("MICROVM_FIRECRACKER_ROOTFS", "/nonexistent/rootfs");
        }
    }

    // Fill the single SANDBOX_MAX_COUNT slot via the mock TEE backend
    // (needs no Docker / Firecracker). The TEE path never dispatches to
    // firecracker::create_and_start, so the pool must stay untouched here.
    let mock = MockTeeBackend::new(TeeType::Tdx);
    let filler = CreateSandboxParams {
        name: "filler".into(),
        image: "test:latest".into(),
        tee_config: Some(TeeConfig {
            required: true,
            tee_type: TeeType::Tdx,
            attestation_nonce: None,
        }),
        owner: "0xwarm".into(),
        cpu_cores: 1,
        memory_mb: 512,
        ..Default::default()
    };
    create_sidecar(&filler, Some(&mock))
        .await
        .expect("filler create must be admitted");
    assert!(
        !sandbox_runtime::firecracker::warm_pool_initialized_for_tests(),
        "TEE create must not touch the firecracker warm pool"
    );

    // A firecracker create over the count cap must be rejected by admission
    // BEFORE the warm pool is consulted: the engine (and its seeding) must
    // never have been constructed.
    let fc_request = CreateSandboxParams {
        name: "fc-over-cap".into(),
        image: String::new(),
        metadata_json: r#"{"runtime_backend":"firecracker"}"#.into(),
        owner: "0xwarm".into(),
        cpu_cores: 1,
        memory_mb: 512,
        ..Default::default()
    };
    let err = create_sidecar(&fc_request, None)
        .await
        .expect_err("create over SANDBOX_MAX_COUNT must be rejected");
    expect_unavailable(err, "Sandbox limit reached");

    assert!(
        !sandbox_runtime::firecracker::warm_pool_initialized_for_tests(),
        "admission-rejected create must not initialize, seed, or claim from \
         the warm pool — warm claims may only happen after admission"
    );
}
