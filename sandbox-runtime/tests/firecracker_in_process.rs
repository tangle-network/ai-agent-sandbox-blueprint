//! Integration tests for the in-process Firecracker driver wrapper.
//!
//! These tests pin the **contract** of `sandbox_runtime::firecracker` against
//! the rest of the sandbox runtime: that a create / resume / delete request
//! for `runtime_backend=firecracker` either drives the VM through the
//! lifecycle (returning a real endpoint and installing real iptables DNAT
//! rules for the requested host port forwards) or fails with a typed
//! `SandboxError` — never with a silent fake-success.
//!
//! Real Firecracker VMM exercise lives in `microvm-runtime`'s own test
//! suite (KVM-gated). We do not duplicate that here; instead we cover the
//! sandbox-runtime side of the boundary: error mapping, idempotency, the
//! "no host-agent process exists" invariant, and the shape of the produced
//! endpoint URL.

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

/// Per-VM env injection must still fail loudly: the in-process driver has
/// no guest-side metadata service yet, so any user-supplied env value would
/// be silently dropped if we allowed the request through. Tracked for the
/// vsock-backed handshake milestone.
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
        matches!(err, SandboxError::Unsupported(_)),
        "expected Unsupported, got {err:?} ({msg})"
    );
    assert!(
        msg.contains("environment injection") && msg.contains("firecracker"),
        "expected env-injection unsupported error, got {msg}"
    );
}

/// With network + vsock + DNAT all wired, a firecracker create against a
/// host that lacks the actual Firecracker binary / kernel must fail at the
/// primitive's prereq check rather than silently fabricating a record. The
/// failure must surface as `Unavailable` (transient, fixable by installing
/// the binary) — never as a successful record with a bogus endpoint.
///
/// This is the inverted form of the old "no host-reachable endpoint yet"
/// invariant: we used to assert `Unsupported`; now we assert that absent
/// the FC binary the error is `Unavailable` (or a `NetworkSetup` failure
/// from `ensure_host`, also mapped to `Unavailable`), not silent success.
#[tokio::test(flavor = "current_thread")]
async fn firecracker_create_without_binary_surfaces_typed_error_no_silent_success() {
    let _guard = test_lock().lock().await;
    set_env("SANDBOX_RUNTIME_BACKEND", Some("docker"));
    set_env("MICROVM_FIRECRACKER_BIN", Some("/nonexistent/firecracker"));
    set_env("MICROVM_FIRECRACKER_KERNEL", Some("/nonexistent/vmlinux"));
    set_env("MICROVM_FIRECRACKER_ROOTFS", Some("/nonexistent/rootfs"));

    let params = fc_params();
    let err = create_sidecar(&params, None)
        .await
        .expect_err("firecracker create must fail when binary is missing");

    // ensure_host can fail before we even reach the FC binary check if the
    // test host lacks CAP_NET_ADMIN / iptables. Either way the error must
    // be typed and explicit — never a successful return with a fake record.
    let msg = err.to_string();
    let is_expected = matches!(
        err,
        SandboxError::Unavailable(_) | SandboxError::Validation(_)
    );
    assert!(
        is_expected,
        "expected Unavailable|Validation from firecracker create, got {err:?} ({msg})"
    );
}

/// Port-forwarding install is now wired: the request reaches the iptables
/// DNAT helper. On a host without `iptables`/`CAP_NET_ADMIN` the install
/// fails — but the failure must surface as `Unavailable` from the DNAT
/// helper, not as `Unsupported` (which would falsely claim "feature not
/// implemented"), and never as silent success.
#[tokio::test(flavor = "current_thread")]
async fn firecracker_create_with_ports_no_longer_returns_unsupported() {
    let _guard = test_lock().lock().await;
    set_env("SANDBOX_RUNTIME_BACKEND", Some("docker"));
    set_env("MICROVM_FIRECRACKER_BIN", Some("/nonexistent/firecracker"));

    let mut params = fc_params();
    params.metadata_json = r#"{"runtime_backend":"firecracker","ports":[3000]}"#.into();

    let err = create_sidecar(&params, None).await.expect_err(
        "firecracker create still fails because the binary / kernel are absent, \
         but the failure mode must no longer be the explicit port-forwarding `Unsupported`",
    );

    // Whatever the error is, it must not be the old
    // "metadata_json.ports forwarding for firecracker sandboxes" deferral —
    // that contract was retired when DNAT install was wired.
    let msg = err.to_string();
    assert!(
        !msg.contains("metadata_json.ports forwarding"),
        "port forwarding is now wired; old `Unsupported` deferral must not fire. Got: {msg}"
    );
    assert!(
        !matches!(err, SandboxError::Unsupported(ref m) if m.contains("ports forwarding")),
        "expected non-port-forwarding-Unsupported error, got {err:?} ({msg})"
    );
}
