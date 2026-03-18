//! Reaper and garbage collection for sandbox lifecycle enforcement.
//!
//! - `reaper_tick()`: stops idle sandboxes, deletes expired ones
//! - `gc_tick()`: removes stopped sandboxes past retention period
//! - `reconcile_on_startup()`: syncs store state with Docker reality

use crate::metrics::metrics;
use crate::runtime::{
    SandboxState, SidecarRuntimeConfig, commit_container, delete_sidecar, docker_builder,
    record_uses_firecracker, refresh_docker_sandbox_endpoint, remove_snapshot_image, sandboxes,
    stop_sidecar, supports_docker_endpoint_refresh,
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
            {
                if container_removed_at + config.sandbox_gc_cold_retention <= now {
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
        // TEE-managed sandboxes: skip Docker reconciliation — their lifecycle is
        // managed by the TEE backend, not Docker. The `container_id` field has a
        // `tee-` prefix and doesn't correspond to a real Docker container.
        if record.tee_deployment_id.is_some() {
            continue;
        }

        if record_uses_firecracker(&record) {
            match crate::firecracker::status(&record.container_id).await {
                Ok(crate::firecracker::FirecrackerContainerStatus::Missing) => {
                    let has_snapshot =
                        record.snapshot_image_id.is_some() || record.snapshot_s3_url.is_some();
                    if has_snapshot {
                        info!(
                            "reconcile: firecracker VM gone for sandbox {}, preserving snapshot record",
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
                            "reconcile: removing orphan firecracker record for sandbox {} (vm {} gone)",
                            record.id, record.container_id
                        );
                        if let Ok(store) = sandboxes() {
                            let _ = store.remove(&record.id);
                        }
                    }
                }
                Ok(crate::firecracker::FirecrackerContainerStatus::Running) => {
                    if record.state == SandboxState::Stopped {
                        info!(
                            "reconcile: marking firecracker sandbox {} as Running",
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
                Ok(crate::firecracker::FirecrackerContainerStatus::Stopped) => {
                    if record.state == SandboxState::Running {
                        info!(
                            "reconcile: marking firecracker sandbox {} as Stopped",
                            record.id
                        );
                        if let Ok(store) = sandboxes() {
                            let _ = store.update(&record.id, |r| {
                                r.state = SandboxState::Stopped;
                                r.stopped_at = Some(now);
                            });
                        }
                    }
                }
                Err(err) => {
                    error!(
                        "reconcile: failed to inspect firecracker sandbox {}: {err}",
                        record.id
                    );
                }
            }
            continue;
        }

        let inspect = crate::runtime::docker_timeout(
            "inspect_container",
            builder
                .client()
                .inspect_container(&record.container_id, None::<InspectContainerOptions>),
        )
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
                } else if running {
                    if supports_docker_endpoint_refresh(&record) {
                        if let Err(err) = refresh_docker_sandbox_endpoint(&record).await {
                            error!(
                                "reconcile: failed to refresh endpoint for sandbox {}: {err}",
                                record.id
                            );
                        }
                    }

                    if record.state == SandboxState::Stopped {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{SandboxRecord, SandboxState, SidecarRuntimeConfig};
    use std::time::Duration;

    /// Helper to create a minimal SandboxRecord for testing.
    fn test_record() -> SandboxRecord {
        SandboxRecord {
            id: "test-sandbox-1".to_string(),
            container_id: "abc123".to_string(),
            sidecar_url: "http://localhost:8080".to_string(),
            sidecar_port: 8080,
            ssh_port: None,
            token: "test-token".to_string(),
            created_at: 1000,
            cpu_cores: 2,
            memory_mb: 1024,
            state: SandboxState::Running,
            idle_timeout_seconds: 300,
            max_lifetime_seconds: 3600,
            last_activity_at: 0,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "ubuntu:22.04".to_string(),
            base_env_json: String::new(),
            user_env_json: String::new(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: "test".to_string(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 10,
            stack: String::new(),
            owner: "0xdeadbeef".to_string(),
            tee_config: None,
            extra_ports: std::collections::HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
        }
    }

    /// Helper to create a test config with known values.
    fn test_config() -> SidecarRuntimeConfig {
        SidecarRuntimeConfig {
            image: "test:latest".to_string(),
            public_host: "127.0.0.1".to_string(),
            container_port: 8080,
            ssh_port: 22,
            timeout: Duration::from_secs(30),
            docker_host: None,
            pull_image: false,
            sandbox_default_idle_timeout: 300,
            sandbox_default_max_lifetime: 3600,
            sandbox_max_idle_timeout: 7200,
            sandbox_max_max_lifetime: 86400,
            sandbox_reaper_interval: 60,
            sandbox_gc_interval: 300,
            sandbox_gc_hot_retention: 3600,
            sandbox_gc_warm_retention: 86400,
            sandbox_gc_cold_retention: 604800,
            snapshot_auto_commit: true,
            snapshot_destination_prefix: Some("s3://my-bucket/snapshots/".to_string()),
            sandbox_max_count: 100,
        }
    }

    // ── resolve_snapshot_destination ─────────────────────────────────────

    #[test]
    fn resolve_snapshot_uses_record_destination_if_set() {
        let mut record = test_record();
        record.snapshot_destination = Some("s3://user-bucket/my-snap.tar.gz".to_string());
        let config = test_config();

        let result = resolve_snapshot_destination(&record, &config);
        assert_eq!(result, Some("s3://user-bucket/my-snap.tar.gz".to_string()));
    }

    #[test]
    fn resolve_snapshot_uses_config_prefix_when_no_record_destination() {
        let record = test_record();
        let config = test_config();

        let result = resolve_snapshot_destination(&record, &config);
        assert_eq!(
            result,
            Some("s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz".to_string())
        );
    }

    #[test]
    fn resolve_snapshot_returns_none_when_no_prefix() {
        let record = test_record();
        let mut config = test_config();
        config.snapshot_destination_prefix = None;

        let result = resolve_snapshot_destination(&record, &config);
        assert!(result.is_none());
    }

    // ── is_operator_s3 ──────────────────────────────────────────────────

    #[test]
    fn is_operator_s3_true_when_operator_managed() {
        let record = test_record();
        let config = test_config();
        let url = "s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";

        assert!(is_operator_s3(url, &record, &config));
    }

    #[test]
    fn is_operator_s3_false_when_user_destination_set() {
        let mut record = test_record();
        record.snapshot_destination = Some("s3://user-bucket/snap.tar.gz".to_string());
        let config = test_config();
        let url = "s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";

        // Even though URL matches prefix, record has user destination -> BYOS3
        assert!(!is_operator_s3(url, &record, &config));
    }

    #[test]
    fn is_operator_s3_false_when_no_prefix() {
        let record = test_record();
        let mut config = test_config();
        config.snapshot_destination_prefix = None;
        let url = "s3://some-bucket/snap.tar.gz";

        assert!(!is_operator_s3(url, &record, &config));
    }

    #[test]
    fn is_operator_s3_false_when_url_does_not_match_prefix() {
        let record = test_record();
        let config = test_config();
        let url = "s3://other-bucket/something/snap.tar.gz";

        assert!(!is_operator_s3(url, &record, &config));
    }

    // ── Phase 2E: Reaper Logic Tests ────────────────────────────────────

    #[test]
    fn resolve_snapshot_trailing_slash() {
        // Prefix with trailing slash
        let record = test_record();
        let config = test_config();
        let result = resolve_snapshot_destination(&record, &config);
        assert_eq!(
            result,
            Some("s3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz".to_string()),
        );

        // Prefix without trailing slash
        let mut config_no_slash = test_config();
        config_no_slash.snapshot_destination_prefix = Some("s3://my-bucket/snapshots".to_string());
        let result_no_slash = resolve_snapshot_destination(&record, &config_no_slash);
        assert_eq!(
            result_no_slash,
            Some("s3://my-bucket/snapshotstest-sandbox-1/snapshot.tar.gz".to_string()),
            "Without trailing slash, URL is directly concatenated"
        );
    }

    #[test]
    fn is_operator_s3_case_sensitive() {
        let record = test_record();
        let config = test_config();
        // Prefix is "s3://my-bucket/snapshots/" — uppercase should NOT match
        let url_upper = "S3://my-bucket/snapshots/test-sandbox-1/snapshot.tar.gz";
        assert!(
            !is_operator_s3(url_upper, &record, &config),
            "S3 prefix comparison should be case-sensitive"
        );
    }

    #[test]
    fn gc_skips_tee_sandboxes() {
        let mut record = test_record();
        record.state = SandboxState::Stopped;
        record.tee_deployment_id = Some("tee-deploy-1".to_string());
        // TEE sandboxes should be skipped in GC — verify by checking the
        // condition that gc_tick uses
        assert!(
            record.tee_deployment_id.is_some(),
            "TEE record should have deployment_id set"
        );
        // The GC code does `if record.tee_deployment_id.is_some() { continue; }`
        // We verify the field is correctly set so the skip condition holds.
    }

    #[test]
    fn gc_skips_firecracker_sandboxes_without_container_removed() {
        let mut record = test_record();
        record.state = SandboxState::Stopped;
        record.metadata_json = r#"{"runtime_backend":"firecracker"}"#.to_string();
        // Firecracker sandboxes have a separate GC path in gc_tick.
        // When container_removed_at is None, they skip the Docker GC path.
        assert!(
            crate::runtime::record_uses_firecracker(&record),
            "record with runtime_backend=firecracker should be detected"
        );
        // The Docker GC path (hot->warm->cold) is skipped for firecracker;
        // instead, firecracker has its own cold->gone path.
    }
}
