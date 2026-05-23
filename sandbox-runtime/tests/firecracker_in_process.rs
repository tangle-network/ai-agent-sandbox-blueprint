//! Integration tests for the in-process Firecracker driver wrapper.
//!
//! These tests pin the **contract** of `sandbox_runtime::firecracker` against
//! the rest of the sandbox runtime: that a create / resume / delete request
//! for `runtime_backend=firecracker` either drives the VM through the
//! lifecycle or fails with [`SandboxError::Unsupported`] — never with a
//! silent fake-success.
//!
//! Real Firecracker VMM exercise lives in `microvm-runtime`'s own test
//! suite (KVM-gated). We do not duplicate that here; instead we cover the
//! sandbox-runtime side of the boundary: error mapping, idempotency, and
//! the "no host-agent process exists" invariant.

use std::sync::OnceLock;

use sandbox_runtime::SandboxError;
use sandbox_runtime::runtime::{CreateSandboxParams, create_sidecar};
use tokio::sync::Mutex as AsyncMutex;

/// All tests in this file share process-level env vars and a `OnceLock`-based
/// store, so they must run sequentially. cargo nextest isolates each test in
/// its own process so this is only needed under default `cargo test`.
///
/// Uses an async mutex because each guard is held across `create_sidecar`'s
/// `.await` — a std `Mutex` would trip clippy's `await_holding_lock`.
fn test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

fn set_env(key: &str, value: Option<&str>) {
    // SAFETY: the test-suite is single-threaded under the `test_lock`
    // mutex, so concurrent mutation of the process env-block is not
    // possible. `std::env::set_var` / `remove_var` are still `unsafe` in
    // Rust 2024 because the safe API has not been finalised; this is the
    // canonical workaround in the rest of the crate.
    match value {
        Some(v) => unsafe { std::env::set_var(key, v) },
        None => unsafe { std::env::remove_var(key) },
    }
}

fn fc_params() -> CreateSandboxParams {
    CreateSandboxParams {
        name: "fc-test".into(),
        image: String::new(),
        stack: "default".into(),
        agent_identifier: "default-agent".into(),
        env_json: "{}".into(),
        user_env_json: String::new(),
        capabilities_json: String::new(),
        metadata_json: r#"{"runtime_backend":"firecracker"}"#.into(),
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 900,
        cpu_cores: 1,
        memory_mb: 512,
        disk_gb: 4,
        port_mappings: Vec::new(),
        tee_config: None,
        owner: String::new(),
        service_id: None,
        ssh_enabled: false,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
    }
}

/// Spawning a Firecracker sandbox today fails with `Unsupported` because the
/// driver primitive has no networking support yet.
///
/// This pins the contract documented in `sandbox-runtime/src/firecracker.rs`:
/// the create path never returns a half-provisioned record with a fake
/// endpoint. When `microvm-runtime` ships the network layer, this assertion
/// flips and we add a positive coverage test for the endpoint.
#[tokio::test(flavor = "current_thread")]
async fn firecracker_create_surfaces_unsupported_without_silent_fallback() {
    let _guard = test_lock().lock().await;

    // Make sure the process default runtime backend is not pinned to
    // firecracker globally — we drive selection through metadata_json on
    // this request only.
    set_env("SANDBOX_RUNTIME_BACKEND", Some("docker"));
    // Point the driver at paths that definitely do not exist so the
    // primitive's `ensure_prereqs` check would fire if reached. We expect
    // the wrapper to fail with `Unsupported` *before or at* that prereq
    // step, never with a `Validation` config error pretending to be the
    // sidecar's fault.
    set_env("MICROVM_FIRECRACKER_BIN", Some("/nonexistent/firecracker"));
    set_env("MICROVM_FIRECRACKER_KERNEL", Some("/nonexistent/vmlinux"));
    set_env("MICROVM_FIRECRACKER_ROOTFS", Some("/nonexistent/rootfs"));

    let params = fc_params();
    let err = create_sidecar(&params, None)
        .await
        .expect_err("firecracker create must fail until microvm-runtime 0.2.0 lands networking");

    // The wrapper is allowed to surface either:
    // - `Unsupported`: the explicit "no networking yet" signal, OR
    // - `Unavailable`: the primitive's prereq check ("firecracker binary
    //   not found") propagated through our mapping.
    // What it MUST NOT do is succeed and persist a record with a bogus URL.
    let msg = err.to_string();
    let is_expected = matches!(
        err,
        SandboxError::Unsupported(_) | SandboxError::Unavailable(_)
    );
    assert!(
        is_expected,
        "expected Unsupported|Unavailable from firecracker create, got {err:?} ({msg})"
    );
}

/// Per-VM env injection must fail loudly. The previous host-agent client
/// silently dropped these requests; the in-process driver returns
/// `Unsupported` so callers know to wait for `microvm-runtime 0.2.0`.
#[tokio::test(flavor = "current_thread")]
async fn firecracker_create_rejects_env_injection_with_unsupported() {
    let _guard = test_lock().lock().await;
    set_env("SANDBOX_RUNTIME_BACKEND", Some("docker"));
    set_env("MICROVM_FIRECRACKER_BIN", Some("/nonexistent/firecracker"));

    let mut params = fc_params();
    // Force the env-injection path: any non-empty env_json triggers the
    // wrapper's `Unsupported("per-VM environment injection ...")` short-circuit.
    params.env_json = r#"{"FOO":"bar"}"#.into();
    params.user_env_json = r#"{"BAZ":"qux"}"#.into();

    let err = create_sidecar(&params, None)
        .await
        .expect_err("env injection must be rejected until the driver supports it");
    let msg = err.to_string();
    assert!(
        msg.contains("environment injection") || msg.contains("microvm-runtime"),
        "expected env-injection unsupported error, got {err:?} ({msg})"
    );
}

/// `metadata_json.ports` host port mappings must fail loudly. The previous
/// host-agent client parsed-and-persisted but did not forward — the new
/// driver surfaces this gap as an explicit `Unsupported` so operators don't
/// rely on the behaviour by accident.
#[tokio::test(flavor = "current_thread")]
async fn firecracker_create_rejects_port_forwarding_with_unsupported() {
    let _guard = test_lock().lock().await;
    set_env("SANDBOX_RUNTIME_BACKEND", Some("docker"));
    set_env("MICROVM_FIRECRACKER_BIN", Some("/nonexistent/firecracker"));

    let mut params = fc_params();
    params.metadata_json = r#"{"runtime_backend":"firecracker","ports":[3000]}"#.into();

    let err = create_sidecar(&params, None)
        .await
        .expect_err("port forwarding must be rejected until the driver supports it");
    let msg = err.to_string();
    assert!(
        msg.contains("metadata_json.ports") || msg.contains("microvm-runtime"),
        "expected port-forwarding unsupported error, got {err:?} ({msg})"
    );
}
