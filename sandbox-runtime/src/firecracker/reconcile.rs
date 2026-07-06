//! Restart reconciliation: reap orphaned warm-pool Firecracker processes.

use super::*;

/// A warm-pool Firecracker process orphaned by a previous operator process.
pub(crate) struct WarmOrphan {
    pid: i32,
    vm_id: String,
}

/// Reap warm-pool Firecracker processes orphaned by a previous operator
/// process. The pool is process-local: on restart the fresh process's provider
/// maps are empty, so prior `fcwarm-*` templates and `warm-*` pool entries keep
/// running — holding guest memory — with no sandbox record and no in-memory
/// handle to stop them. This finds them from the OS, SIGKILLs them, and
/// releases their by-id host resources.
///
/// MUST run before the first `seed_generation`: a fresh process resets the
/// generation counter to 0 and re-mints the exact same ids, so reaping after a
/// seed would kill the just-seeded generation. Called as the first step of the
/// `WARM_SERVING` init closure, and from `reaper::reconcile_on_startup` (which
/// also covers the "warm disabled now, but a prior process left orphans" case
/// the lazy engine init never reaches).
pub(crate) fn reconcile_warm_orphans() {
    let socket_dir = provider().config.socket_dir.clone();
    // Live sandboxes are never warm-prefixed, so the prefix filter already
    // spares them; skipping stored ids too is belt-and-suspenders.
    let live: std::collections::HashSet<String> = crate::runtime::sandboxes()
        .and_then(|s| s.values())
        .map(|recs| {
            recs.into_iter()
                .flat_map(|r| [r.id, r.container_id])
                .collect()
        })
        .unwrap_or_default();

    for orphan in enumerate_warm_fc_processes(&socket_dir) {
        if live.contains(&orphan.vm_id) {
            continue;
        }
        tracing::warn!(
            pid = orphan.pid,
            vm_id = %orphan.vm_id,
            "reaping orphaned warm-pool firecracker process from a previous operator process"
        );
        reap_pid(orphan.pid);
        release_orphan_resources(&orphan.vm_id);
    }

    reclaim_orphan_rootfs_clones();
}

/// Reclaim leaked warm-template rootfs clones after the process reap.
///
/// A cold sandbox's clone lives under its own id and is released by
/// `delete`'s own-id path even after a restart, so it never leaks. Only the
/// aliased warm *template* clone (`fcwarm-*` / `warm-*`) can be orphaned — its
/// template process is destroyed at claim, so the `/proc` scan never sees it,
/// and if the owning sandbox's delete didn't run (crash, or a failed lineage
/// persist) its clone is stranded. Reclaim a warm-prefixed clone dir iff no
/// live sandbox's persisted lineage still references it.
///
/// Scoping to warm-prefixed dirs is the data-loss guard: a live sandbox's
/// own-id clone is never warm-prefixed, so this can never delete a live
/// workspace out from under a running sandbox.
pub(crate) fn reclaim_orphan_rootfs_clones() {
    let referenced: std::collections::HashSet<String> =
        crate::firecracker_lineage::referenced_template_ids()
            .into_iter()
            .collect();
    let clones_dir = rootfs_registry().config().clones_dir.clone();
    let Ok(entries) = std::fs::read_dir(&clones_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if should_reclaim_clone(&id, &referenced) {
            tracing::warn!(
                clone_id = %id,
                "reclaiming orphaned warm-template rootfs clone (no live sandbox lineage references it)"
            );
            let _ = rootfs_registry().release(&id);
        }
    }
}

/// A `clones_dir` entry is reclaimable iff it is a warm-template clone
/// (`fcwarm-*` / `warm-*`) that no live sandbox's lineage still references. A
/// non-warm-prefixed id is NEVER reclaimed — that is a live sandbox's own-id
/// clone, and removing it would destroy a running workspace. This predicate is
/// the data-loss guard for the sweep.
pub(crate) fn should_reclaim_clone(
    id: &str,
    referenced: &std::collections::HashSet<String>,
) -> bool {
    is_warm_vm_id(id) && !referenced.contains(id)
}

/// Enumerate live Firecracker processes whose API socket sits under our
/// `socket_dir` and whose vm_id carries a warm-pool prefix. Reads only `/proc`
/// — the provider's in-memory maps are empty in a freshly started process, so a
/// prior process's VMs are invisible to it and must be found from the OS.
pub(crate) fn enumerate_warm_fc_processes(socket_dir: &std::path::Path) -> Vec<WarmOrphan> {
    let mut orphans = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return orphans;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        // cmdline is NUL-separated argv.
        let args: Vec<&str> = cmdline
            .split(|b| *b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .filter(|s| !s.is_empty())
            .collect();
        let Some(sock) = args
            .iter()
            .position(|a| *a == "--api-sock")
            .and_then(|i| args.get(i + 1))
        else {
            continue;
        };
        // Socket layout is `<socket_dir>/<vm_id>/api.sock`; require the socket
        // to live under OUR socket_dir so we never touch an unrelated VMM.
        let sock_path = std::path::Path::new(sock);
        if sock_path.parent().and_then(|p| p.parent()) != Some(socket_dir) {
            continue;
        }
        let Some(vm_id) = sock_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        else {
            continue;
        };
        // A warm CLAIM (`rename_vm`) moves the socket dir on disk but cannot
        // rewrite a running FC's `--api-sock` argv, so a LIVE claimed sandbox
        // still shows the warm-entry path in `/proc/cmdline`. Its socket file
        // has moved and no longer exists at that path; only an unclaimed orphan
        // still has its socket there. Skipping the missing-socket case is what
        // prevents reconcile from SIGKILLing a live warm-claimed sandbox after
        // an operator restart — data loss, not a leak.
        if is_warm_vm_id(vm_id) && sock_path.exists() {
            orphans.push(WarmOrphan {
                pid,
                vm_id: vm_id.to_string(),
            });
        }
    }
    orphans
}

/// A warm-pool vm_id: `fcwarm-*` templates/riders or `warm-*` pool entries
/// (`microvm-warm-pool` names entries `warm-<stack>-<ver>-<seed>`). Production
/// sandbox ids are session UUIDs and never carry these prefixes, so this is the
/// primary guard against reaping a live sandbox.
pub(crate) fn is_warm_vm_id(vm_id: &str) -> bool {
    vm_id.starts_with("fcwarm-") || vm_id.starts_with("warm-")
}

/// SIGKILL a pid via the `kill` utility. The orphan was reparented to init when
/// its parent (the previous operator process) exited, so it is not our child
/// and cannot be `waitpid`-ed; init reaps the zombie. `libc` is gated behind a
/// TEE feature in this crate and the Firecracker path is Linux-only, so shell
/// out rather than widen the default dependency set.
pub(crate) fn reap_pid(pid: i32) {
    match std::process::Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            tracing::warn!(
                pid,
                ?status,
                "kill -KILL of orphaned firecracker returned non-zero"
            )
        }
        Err(err) => tracing::warn!(pid, %err, "failed to signal orphaned firecracker process"),
    }
}

/// Release the by-id host resources an orphaned warm VM held. Every manager
/// derives its kernel/host object from the vm_id (TAP = `tap-<hash(id)>`, vsock
/// UDS = `uds_path_for(id)`, rootfs clone = `clones_dir/id`, DNAT chain =
/// `hash(id)`), so release works without the in-memory record a fresh process
/// lacks. All detaches are best-effort and idempotent.
pub(crate) fn release_orphan_resources(vm_id: &str) {
    release_attachments(
        vm_id,
        &VmAttachments {
            network_attached: true,
            vsock_attached: true,
            dnat_rule_count: 1,
            rootfs_cloned: true,
            warm: None,
        },
    );
    // A template (`fcwarm-g<N>-tpl`) rides a sibling rider TAP under
    // `fcwarm-g<N>-rider`; that host interface has no FC process of its own, so
    // enumeration never sees it — derive and detach it here.
    if let Some(base) = vm_id.strip_suffix("-tpl") {
        let rider_id = format!("{base}-rider");
        if let Err(err) = network().detach(&rider_id) {
            tracing::warn!(
                vm_id,
                rider_id,
                ?err,
                "failed to detach orphaned warm rider TAP"
            );
        }
    }
    // On-disk residue: API socket dir + persisted vmstate dir.
    let _ = std::fs::remove_dir_all(provider().config.socket_dir.join(vm_id));
    let _ = std::fs::remove_dir_all(provider().vm_state_path(vm_id));
}
