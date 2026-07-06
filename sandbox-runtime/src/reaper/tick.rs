use super::*;

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

    for mut record in records {
        if let Err(e) = crate::runtime::unseal_record(&mut record) {
            tracing::error!(id = %record.id, error = %e, "Failed to unseal record in reaper — skipping");
            continue;
        }
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

            // Post-stop: docker commit to preserve filesystem.
            // TEE sandboxes have no Docker container to commit — skip.
            if config.snapshot_auto_commit
                && record.tee_deployment_id.is_none()
                && !record_uses_firecracker(&record)
            {
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
