use super::*;

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

        // TEE sandboxes have no Docker container/image/S3 tier — their lifecycle
        // is managed by the TEE backend, not Docker GC.
        if record.tee_deployment_id.is_some() {
            continue;
        }

        if record_uses_firecracker(&record) {
            if let (Some(container_removed_at), Some(s3_url)) =
                (record.container_removed_at, &record.snapshot_s3_url)
                && container_removed_at + config.sandbox_gc_cold_retention <= now
            {
                let is_operator_managed = is_operator_s3(s3_url, &record, config);
                if is_operator_managed {
                    info!(
                        "gc: firecracker cold->gone for sandbox {} (deleting S3 snapshot)",
                        record.id
                    );
                    if let Err(err) = delete_s3_snapshot(s3_url).await {
                        error!(
                            "gc: failed to delete firecracker S3 snapshot for sandbox {}: {err}",
                            record.id
                        );
                    }
                    metrics().record_gc_s3_cleaned();
                } else {
                    info!(
                        "gc: removing firecracker record for sandbox {} (user BYOS3 preserved at {s3_url})",
                        record.id
                    );
                }
                if let Ok(store) = sandboxes() {
                    let _ = store.remove(&record.id);
                }
                metrics().record_garbage_collected();
                continue;
            }

            if record.container_removed_at.is_some()
                && record.snapshot_image_id.is_none()
                && record.snapshot_s3_url.is_none()
            {
                info!(
                    "gc: cleaning up empty firecracker record for sandbox {}",
                    record.id
                );
                if let Ok(store) = sandboxes() {
                    let _ = store.remove(&record.id);
                }
                metrics().record_garbage_collected();
                continue;
            }
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
            && container_removed_at + config.sandbox_gc_warm_retention <= now
        {
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

        // Tier 3: Cold -> Gone (remove S3 snapshot, remove record)
        if record.snapshot_image_id.is_none()
            && let (Some(s3_url), Some(image_removed_at)) =
                (&record.snapshot_s3_url, record.image_removed_at)
            && image_removed_at + config.sandbox_gc_cold_retention <= now
        {
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
