//! Restart reconcile must SPARE a live warm-CLAIMED sandbox.
//!
//! A warm claim (`rename_vm`) moves the pooled VM's socket dir on disk from the
//! warm-entry id to the sandbox id, but it cannot rewrite the already-exec'd
//! Firecracker's `--api-sock` argv — so a live claimed sandbox still shows the
//! warm-entry (warm-prefixed) path in `/proc/<pid>/cmdline`. A reconcile that
//! reaps by cmdline id alone would SIGKILL that live customer sandbox on the
//! next operator restart — data loss, strictly worse than the leak reconcile
//! exists to fix.
//!
//! Named bug it catches: `enumerate_warm_fc_processes` classifying a claimed
//! sandbox as an orphan. The discriminator is that a claim MOVED the socket, so
//! the cmdline path no longer exists on disk; only an unclaimed orphan still has
//! its socket there.
//!
//! Own test binary: `provider()` builds its config from env once per process.

#![cfg(feature = "test-utils")]

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

fn spawn_idle_fc(bin: &str, sock: &Path) -> Child {
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    let child = Command::new(bin)
        .arg("--api-sock")
        .arg(sock)
        .spawn()
        .expect("spawn firecracker");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sock.exists(), "firecracker never created its api socket");
    child
}

#[test]
fn reconcile_spares_claimed_warm_sandbox_whose_socket_moved() {
    let Some(bin) = fc_binary() else {
        eprintln!("SKIP: no firecracker binary (set FC_E2E_BIN)");
        return;
    };

    let tmp = tempfile::Builder::new()
        .prefix("fcwarm-claim")
        .tempdir_in("/tmp")
        .unwrap();
    let socket_dir = tmp.path().join("sk");
    std::fs::create_dir_all(&socket_dir).unwrap();

    {
        let _guard = sandbox_runtime::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: set before the first provider()/config load in this process.
        unsafe {
            std::env::set_var("MICROVM_FIRECRACKER_BIN", &bin);
            std::env::set_var("MICROVM_FIRECRACKER_SOCKET_DIR", &socket_dir);
            std::env::set_var("MICROVM_FIRECRACKER_STATE_DIR", tmp.path().join("st"));
            std::env::set_var("BLUEPRINT_STATE_DIR", tmp.path().join("store"));
            std::env::set_var("SIDECAR_IMAGE", "test:latest");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        }
    }

    // A pooled warm entry, spawned at the warm-prefixed socket path.
    let entry_sock = socket_dir.join("warm-stack-1_0_0-seed").join("api.sock");
    let mut claimed = spawn_idle_fc(&bin, &entry_sock);

    // Simulate the claim: rename_vm moves the socket dir to the sandbox id.
    // The FC keeps serving on the moved socket (the inode followed the rename);
    // its /proc/cmdline still shows the warm-entry path, which is now gone.
    std::fs::rename(
        socket_dir.join("warm-stack-1_0_0-seed"),
        socket_dir.join("sandbox-claimed-xyz"),
    )
    .expect("simulate claim rename");
    assert!(
        !socket_dir.join("warm-stack-1_0_0-seed").exists(),
        "the warm-entry socket path must be gone after the claim rename"
    );
    assert!(
        claimed.try_wait().unwrap().is_none(),
        "the claimed FC must be alive before reconcile"
    );

    // Restart reconcile with an empty store: the claimed sandbox MUST survive.
    sandbox_runtime::firecracker::reconcile_warm_orphans_for_tests();

    std::thread::sleep(Duration::from_millis(300));
    assert!(
        claimed.try_wait().unwrap().is_none(),
        "reconcile SIGKILLed a LIVE warm-claimed sandbox (cmdline shows the moved \
         warm-entry path) — data loss; enumerate must skip processes whose socket moved"
    );

    let _ = claimed.kill();
    let _ = claimed.wait();
}
