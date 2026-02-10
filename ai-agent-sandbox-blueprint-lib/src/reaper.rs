//! Reaper and garbage collection for sandbox lifecycle enforcement.
//!
//! - `reaper_tick()`: stops idle sandboxes, deletes expired ones
//! - `gc_tick()`: removes stopped sandboxes past retention period
//! - `reconcile_on_startup()`: syncs store state with Docker reality

use crate::metrics::metrics;
use crate::runtime::{
    SandboxState, SidecarRuntimeConfig, delete_sidecar, docker_builder, sandboxes, stop_sidecar,
};
use blueprint_sdk::{error, info};
use docktopus::bollard::container::InspectContainerOptions;

/// Enforce idle timeout and max lifetime on running sandboxes.
///
/// Called every `SANDBOX_REAPER_INTERVAL` seconds.
/// - If `created_at + max_lifetime <= now` → hard delete (container removed + record purged)
/// - Else if `last_activity_at + idle_timeout <= now` → soft stop (container stopped, record kept)
/// - Backward compat: if `last_activity_at == 0`, falls back to `created_at`
pub async fn reaper_tick() {
    let now = crate::workflows::now_ts();

    let records = match sandboxes().and_then(|s| s.values()) {
        Ok(v) => v,
        Err(err) => {
            error!("reaper: failed to read sandboxes: {err}");
            return;
        }
    };

    for record in records {
        if record.state != SandboxState::Running {
            continue;
        }

        let activity = if record.last_activity_at > 0 {
            record.last_activity_at
        } else {
            record.created_at
        };

        // Hard kill: exceeded max lifetime
        if record.max_lifetime_seconds > 0
            && record.created_at + record.max_lifetime_seconds <= now
        {
            info!(
                "reaper: deleting sandbox {} (exceeded max lifetime {}s)",
                record.id, record.max_lifetime_seconds
            );
            if let Err(err) = delete_sidecar(&record).await {
                error!("reaper: failed to delete sandbox {}: {err}", record.id);
                continue;
            }
            if let Ok(store) = sandboxes() {
                let _ = store.remove(&record.id);
            }
            metrics().record_reaped_lifetime();
            continue;
        }

        // Soft stop: idle too long
        if record.idle_timeout_seconds > 0
            && activity + record.idle_timeout_seconds <= now
        {
            info!(
                "reaper: stopping sandbox {} (idle for {}s, timeout {}s)",
                record.id,
                now.saturating_sub(activity),
                record.idle_timeout_seconds
            );
            if let Err(err) = stop_sidecar(&record).await {
                error!("reaper: failed to stop sandbox {}: {err}", record.id);
                continue;
            }
            metrics().record_reaped_idle();
        }
    }
}

/// Remove stopped sandboxes that have exceeded the retention period.
///
/// Called every `SANDBOX_GC_INTERVAL` seconds.
pub async fn gc_tick() {
    let config = SidecarRuntimeConfig::load();
    let retention = config.sandbox_gc_stopped_retention;
    let now = crate::workflows::now_ts();

    let records = match sandboxes().and_then(|s| s.values()) {
        Ok(v) => v,
        Err(err) => {
            error!("gc: failed to read sandboxes: {err}");
            return;
        }
    };

    for record in records {
        if record.state != SandboxState::Stopped {
            continue;
        }

        let stopped_at = match record.stopped_at {
            Some(ts) => ts,
            None => continue,
        };

        if stopped_at + retention <= now {
            info!(
                "gc: deleting stopped sandbox {} (stopped {}s ago, retention {}s)",
                record.id,
                now.saturating_sub(stopped_at),
                retention
            );
            if let Err(err) = delete_sidecar(&record).await {
                error!("gc: failed to delete sandbox {}: {err}", record.id);
                continue;
            }
            if let Ok(store) = sandboxes() {
                let _ = store.remove(&record.id);
            }
            metrics().record_garbage_collected();
        }
    }
}

/// Reconcile stored sandbox state with Docker reality on startup.
///
/// - Container gone → remove orphan record
/// - Container stopped but record says Running → update to Stopped
/// - Container running but record says Stopped → update to Running
pub async fn reconcile_on_startup() {
    let builder = match docker_builder().await {
        Ok(b) => b,
        Err(err) => {
            error!("reconcile: failed to connect to Docker: {err}");
            return;
        }
    };

    let records = match sandboxes().and_then(|s| s.values()) {
        Ok(v) => v,
        Err(err) => {
            error!("reconcile: failed to read sandboxes: {err}");
            return;
        }
    };

    let now = crate::workflows::now_ts();

    for record in records {
        let inspect = builder
            .client()
            .inspect_container(&record.container_id, None::<InspectContainerOptions>)
            .await;

        match inspect {
            Err(_) => {
                // Container doesn't exist — remove orphan record
                info!(
                    "reconcile: removing orphan record for sandbox {} (container {} gone)",
                    record.id, record.container_id
                );
                if let Ok(store) = sandboxes() {
                    let _ = store.remove(&record.id);
                }
            }
            Ok(info) => {
                let running = info
                    .state
                    .as_ref()
                    .and_then(|s| s.running)
                    .unwrap_or(false);

                if !running && record.state == SandboxState::Running {
                    info!(
                        "reconcile: marking sandbox {} as Stopped (container not running)",
                        record.id
                    );
                    if let Ok(store) = sandboxes() {
                        let _ = store.update(&record.id, |r| {
                            r.state = SandboxState::Stopped;
                            r.stopped_at = Some(now);
                        });
                    }
                } else if running && record.state == SandboxState::Stopped {
                    info!(
                        "reconcile: marking sandbox {} as Running (container is running)",
                        record.id
                    );
                    if let Ok(store) = sandboxes() {
                        let _ = store.update(&record.id, |r| {
                            r.state = SandboxState::Running;
                            r.stopped_at = None;
                        });
                    }
                }
            }
        }
    }
}
