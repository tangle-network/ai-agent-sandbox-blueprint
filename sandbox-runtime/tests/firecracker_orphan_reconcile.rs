//! Restart-leak reconcile: warm-pool Firecracker processes orphaned by a
//! previous operator process must be reaped on startup. The pool is
//! process-local, so a fresh process's provider maps are empty and prior
//! `fcwarm-*` / `warm-*` VMs would keep running (holding guest memory) with no
//! sandbox record. `firecracker::reconcile_warm_orphans` finds them from
//! `/proc` and SIGKILLs them, releasing their by-id host resources.
//!
//! Named bug it catches: no startup reconcile — or one that keys off the
//! in-memory provider map (empty after restart) — so the orphaned Firecracker
//! process survives and its guest memory leaks. The negative half also catches
//! an over-broad reconcile that reaps a live (non-warm) sandbox's VM.
//!
//! Real Firecracker, no /dev/kvm and no root needed: an idle `firecracker
//! --api-sock` serves its API socket without booting a guest, and the test
//! signals only its own child processes. Self-skips when no FC binary exists.

#![cfg(feature = "test-utils")]

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn fc_binary() -> Option<String> {
    let candidate = std::env::var("FC_E2E_BIN").unwrap_or_else(|_| {
        format!(
            "{}/.local/bin/firecracker",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    Path::new(&candidate).exists().then_some(candidate)
}

/// Spawn an idle Firecracker whose API socket lives at
/// `<socket_dir>/<vm_id>/api.sock`, and wait for the socket to appear.
fn spawn_idle_fc(bin: &str, socket_dir: &Path, vm_id: &str) -> Child {
    let dir = socket_dir.join(vm_id);
    std::fs::create_dir_all(&dir).expect("mk socket dir");
    let sock = dir.join("api.sock");
    let child = Command::new(bin)
        .arg("--api-sock")
        .arg(&sock)
        .spawn()
        .expect("spawn firecracker");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sock.exists(), "firecracker never created its api socket");
    child
}

/// Wait (bounded) for a child to terminate; true if it exited via signal
/// (the reconcile's SIGKILL) rather than still running.
fn wait_killed(child: &mut Child, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.signal().is_some() || !status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => return false,
            Err(_) => return false,
        }
    }
}

#[test]
fn reconcile_reaps_orphan_warm_fc_but_spares_live_sandbox() {
    let Some(bin) = fc_binary() else {
        eprintln!("SKIP: no firecracker binary (set FC_E2E_BIN)");
        return;
    };

    // Short base path: unix socket paths cap at ~108 bytes, and the scratchpad
    // tempdir is already long — put ours under /tmp so the api.sock fits.
    let tmp = tempfile::Builder::new()
        .prefix("fcwarm-rec")
        .tempdir_in("/tmp")
        .unwrap();
    let socket_dir = tmp.path().join("sk");
    let state_dir = tmp.path().join("st");
    let store_dir = tmp.path().join("store");
    std::fs::create_dir_all(&socket_dir).unwrap();

    {
        let _guard = sandbox_runtime::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: set before the first provider()/config load in this process.
        unsafe {
            std::env::set_var("MICROVM_FIRECRACKER_BIN", &bin);
            std::env::set_var("MICROVM_FIRECRACKER_SOCKET_DIR", &socket_dir);
            std::env::set_var("MICROVM_FIRECRACKER_STATE_DIR", &state_dir);
            std::env::set_var("BLUEPRINT_STATE_DIR", &store_dir);
            std::env::set_var("SIDECAR_IMAGE", "test:latest");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        }
    }

    // A warm-pool template orphaned by a "previous process", plus a live
    // sandbox's FC (session-shaped id) that reconcile must spare.
    let mut warm = spawn_idle_fc(&bin, &socket_dir, "fcwarm-g0-tpl");
    let mut live = spawn_idle_fc(&bin, &socket_dir, "sandbox-live-abc123");

    // On-disk residue a template leaves behind.
    let warm_state = state_dir.join("fcwarm-g0-tpl");
    std::fs::create_dir_all(&warm_state).unwrap();
    std::fs::write(warm_state.join("vmstate"), b"x").unwrap();

    assert!(
        warm.try_wait().unwrap().is_none(),
        "warm orphan should be alive pre-reconcile"
    );
    assert!(
        live.try_wait().unwrap().is_none(),
        "live fc should be alive pre-reconcile"
    );

    // Restart reconcile with an empty store.
    sandbox_runtime::firecracker::reconcile_warm_orphans_for_tests();

    // The warm orphan is killed; its socket + state residue is removed.
    assert!(
        wait_killed(&mut warm, Duration::from_secs(5)),
        "reconcile must SIGKILL the orphaned warm firecracker process"
    );
    assert!(
        !socket_dir.join("fcwarm-g0-tpl").exists(),
        "warm socket residue must be removed"
    );
    assert!(!warm_state.exists(), "warm state residue must be removed");

    // The live sandbox's FC is untouched.
    assert!(
        live.try_wait().unwrap().is_none(),
        "reconcile must NOT touch a non-warm (live-sandbox) firecracker"
    );
    assert!(
        socket_dir.join("sandbox-live-abc123").exists(),
        "live socket dir must be preserved"
    );

    let _ = live.kill();
    let _ = live.wait();
}
