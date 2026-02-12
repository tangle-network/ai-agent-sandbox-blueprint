//! Reaper and garbage collection for sandbox lifecycle enforcement.
//!
//! - `reaper_tick()`: stops idle sandboxes, deletes expired ones
//! - `gc_tick()`: removes stopped sandboxes past retention period
//! - `reconcile_on_startup()`: syncs store state with Docker reality

use crate::metrics::metrics;
use crate::runtime::{
    SandboxState, SidecarRuntimeConfig, commit_container, delete_sidecar, docker_builder,
    remove_snapshot_image, sandboxes, stop_sidecar,
};
use blueprint_sdk::{error, info};
use docktopus::bollard::container::InspectContainerOptions;

/// Enforce idle timeout and max lifetime on running sandboxes.
///
/// Called every `SANDBOX_REAPER_INTERVAL` seconds.
pub async fn reaper_tick() {
    let now = crate::util::now_ts();

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
        if record.max_lifetime_seconds > 0 && record.created_at + record.max_lifetime_seconds <= now
        {
            info!(
                "reaper: deleting sandbox {} (exceeded max lifetime {}s)",
                record.id, record.max_lifetime_seconds
            );
            if let Err(err) = delete_sidecar(&record, None).await {
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
        if record.idle_timeout_seconds > 0 && activity + record.idle_timeout_seconds <= now {
            info!(
                "reaper: stopping sandbox {} (idle for {}s, timeout {}s)",
                record.id,
                now.saturating_sub(activity),
                record.idle_timeout_seconds
            );

            let config = SidecarRuntimeConfig::load();

            // Pre-stop: upload S3 snapshot while container is still running
            let snapshot_dest = resolve_snapshot_destination(&record, config);
            if let Some(ref dest) = snapshot_dest {
                match upload_s3_snapshot(&record, dest).await {
                    Ok(()) => {
                        if let Ok(store) = sandboxes() {
                            let dest_clone = dest.clone();
                            let _ = store.update(&record.id, |r| {
                                r.snapshot_s3_url = Some(dest_clone);
                            });
                        }
                        metrics().record_snapshot_uploaded();
                        info!("reaper: uploaded S3 snapshot for sandbox {}", record.id);
                    }
                    Err(err) => {
                        error!(
                            "reaper: S3 snapshot upload failed for sandbox {}: {err}",
                            record.id
                        );
                    }
                }
            }

            // Stop the container
            if let Err(err) = stop_sidecar(&record).await {
                error!("reaper: failed to stop sandbox {}: {err}", record.id);
                continue;
            }

            // Post-stop: docker commit to preserve filesystem
            if config.snapshot_auto_commit {
                match commit_container(&record).await {
                    Ok(image_id) => {
                        if let Ok(store) = sandboxes() {
                            let _ = store.update(&record.id, |r| {
                                r.snapshot_image_id = Some(image_id);
                            });
                        }
                        metrics().record_snapshot_committed();
                        info!("reaper: committed snapshot for sandbox {}", record.id);
                    }
                    Err(err) => {
                        error!(
                            "reaper: docker commit failed for sandbox {}: {err}",
                            record.id
                        );
                    }
                }
            }

            metrics().record_reaped_idle();
        }
    }
}

/// Tiered garbage collection for stopped sandboxes.
///
/// Progressively moves sandboxes through storage tiers:
///   Hot (stopped container) -> Warm (committed image) -> Cold (S3 snapshot) -> Gone
///
/// Each tier has a configurable retention period. User BYOS3 copies are never deleted.
///
/// Called every `SANDBOX_GC_INTERVAL` seconds.
pub async fn gc_tick() {
    let config = SidecarRuntimeConfig::load();
    let now = crate::util::now_ts();

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

        // Tier 1: Hot -> Warm (remove container, keep committed image)
        if record.container_removed_at.is_none()
            && stopped_at + config.sandbox_gc_hot_retention <= now
        {
            let has_snapshot =
                record.snapshot_image_id.is_some() || record.snapshot_s3_url.is_some();

            if has_snapshot {
                info!(
                    "gc: hot->warm for sandbox {} (removing container, keeping snapshot)",
                    record.id
                );
                if let Err(err) = delete_sidecar(&record, None).await {
                    error!(
                        "gc: failed to remove container for sandbox {}: {err}",
                        record.id
                    );
                    continue;
                }
                if let Ok(store) = sandboxes() {
                    let _ = store.update(&record.id, |r| {
                        r.container_removed_at = Some(now);
                    });
                }
                metrics().record_gc_container_removed();
            } else {
                // No snapshot at all — full cleanup (legacy behavior)
                info!(
                    "gc: deleting sandbox {} (no snapshot, stopped {}s ago)",
                    record.id,
                    now.saturating_sub(stopped_at)
                );
                if let Err(err) = delete_sidecar(&record, None).await {
                    error!("gc: failed to delete sandbox {}: {err}", record.id);
                    continue;
                }
                if let Ok(store) = sandboxes() {
                    let _ = store.remove(&record.id);
                }
                metrics().record_garbage_collected();
            }
            continue;
        }

        // Tier 2: Warm -> Cold (remove committed image, keep S3)
        if let (Some(container_removed_at), Some(image_id)) =
            (record.container_removed_at, &record.snapshot_image_id)
        {
            if container_removed_at + config.sandbox_gc_warm_retention <= now {
                info!(
                    "gc: warm->cold for sandbox {} (removing image {})",
                    record.id, image_id
                );
                if let Err(err) = remove_snapshot_image(image_id).await {
                    error!(
                        "gc: failed to remove snapshot image for sandbox {}: {err}",
                        record.id
                    );
                }
                if let Ok(store) = sandboxes() {
                    let _ = store.update(&record.id, |r| {
                        r.snapshot_image_id = None;
                        r.image_removed_at = Some(now);
                    });
                }
                metrics().record_gc_image_removed();

                // If no S3 snapshot exists, remove record entirely
                if record.snapshot_s3_url.is_none() {
                    if let Ok(store) = sandboxes() {
                        let _ = store.remove(&record.id);
                    }
                    metrics().record_garbage_collected();
                }
                continue;
            }
        }

        // Tier 3: Cold -> Gone (remove S3 snapshot, remove record)
        if record.snapshot_image_id.is_none() {
            if let (Some(s3_url), Some(image_removed_at)) =
                (&record.snapshot_s3_url, record.image_removed_at)
            {
                if image_removed_at + config.sandbox_gc_cold_retention <= now {
                    // Only delete operator-managed S3 snapshots, not user BYOS3
                    let is_operator_managed = is_operator_s3(s3_url, &record, config);
                    if is_operator_managed {
                        info!(
                            "gc: cold->gone for sandbox {} (deleting S3 snapshot)",
                            record.id
                        );
                        if let Err(err) = delete_s3_snapshot(s3_url).await {
                            error!(
                                "gc: failed to delete S3 snapshot for sandbox {}: {err}",
                                record.id
                            );
                        }
                        metrics().record_gc_s3_cleaned();
                    } else {
                        info!(
                            "gc: removing record for sandbox {} (user BYOS3 preserved at {s3_url})",
                            record.id
                        );
                    }
                    if let Ok(store) = sandboxes() {
                        let _ = store.remove(&record.id);
                    }
                    metrics().record_garbage_collected();
                    continue;
                }
            }
        }

        // Cleanup: record has no container, no image, no S3 -> remove
        if record.container_removed_at.is_some()
            && record.snapshot_image_id.is_none()
            && record.snapshot_s3_url.is_none()
        {
            info!("gc: cleaning up empty record for sandbox {}", record.id);
            if let Ok(store) = sandboxes() {
                let _ = store.remove(&record.id);
            }
            metrics().record_garbage_collected();
        }
    }
}

/// Reconcile stored sandbox state with Docker reality on startup.
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

    let now = crate::util::now_ts();

    for record in records {
        let inspect = builder
            .client()
            .inspect_container(&record.container_id, None::<InspectContainerOptions>)
            .await;

        match inspect {
            Err(_) => {
                let has_snapshot =
                    record.snapshot_image_id.is_some() || record.snapshot_s3_url.is_some();
                if has_snapshot {
                    info!(
                        "reconcile: container gone for sandbox {}, preserving snapshot record",
                        record.id
                    );
                    if let Ok(store) = sandboxes() {
                        let _ = store.update(&record.id, |r| {
                            r.state = SandboxState::Stopped;
                            if r.stopped_at.is_none() {
                                r.stopped_at = Some(now);
                            }
                            if r.container_removed_at.is_none() {
                                r.container_removed_at = Some(now);
                            }
                        });
                    }
                } else {
                    info!(
                        "reconcile: removing orphan record for sandbox {} (container {} gone)",
                        record.id, record.container_id
                    );
                    if let Ok(store) = sandboxes() {
                        let _ = store.remove(&record.id);
                    }
                }
            }
            Ok(info) => {
                let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);

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

/// Resolve the snapshot destination URL for a sandbox.
fn resolve_snapshot_destination(
    record: &crate::runtime::SandboxRecord,
    config: &SidecarRuntimeConfig,
) -> Option<String> {
    if let Some(ref dest) = record.snapshot_destination {
        return Some(dest.clone());
    }
    config
        .snapshot_destination_prefix
        .as_ref()
        .map(|prefix| format!("{}{}/snapshot.tar.gz", prefix, record.id))
}

/// Upload a snapshot of the running container's workspace to S3/HTTP via sidecar exec.
async fn upload_s3_snapshot(
    record: &crate::runtime::SandboxRecord,
    destination: &str,
) -> std::result::Result<(), String> {
    let command =
        crate::util::build_snapshot_command(destination, true, true).map_err(|e| e.to_string())?;
    let payload = serde_json::json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&command)),
    });
    crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Check if an S3 URL is operator-managed (not user BYOS3).
fn is_operator_s3(
    s3_url: &str,
    record: &crate::runtime::SandboxRecord,
    config: &SidecarRuntimeConfig,
) -> bool {
    if record.snapshot_destination.is_some() {
        return false;
    }
    if let Some(ref prefix) = config.snapshot_destination_prefix {
        return s3_url.starts_with(prefix.as_str());
    }
    false
}

/// Best-effort DELETE of an S3/HTTP snapshot URL via reqwest.
async fn delete_s3_snapshot(url: &str) -> std::result::Result<(), String> {
    let client = crate::util::http_client().map_err(|e| e.to_string())?;
    let resp = client
        .delete(url)
        .send()
        .await
        .map_err(|e| format!("S3 delete request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("S3 delete returned status {}", resp.status()));
    }
    Ok(())
}
