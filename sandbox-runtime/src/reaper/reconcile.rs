use super::*;

/// Reconcile stored sandbox state with Docker reality on startup.
pub async fn reconcile_on_startup() {
    // Reap warm-pool VMs orphaned by a previous process first — before the
    // Docker connect below (which may legitimately fail on a Firecracker-only
    // host) and before any create can seed a fresh generation. Covers the
    // "warm disabled now, but a prior process left orphans" case the lazy
    // engine init never reaches.
    crate::firecracker::reconcile_warm_orphans();

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
                    if supports_docker_endpoint_refresh(&record)
                        && let Err(err) = refresh_docker_sandbox_endpoint(&record).await
                    {
                        error!(
                            "reconcile: failed to refresh endpoint for sandbox {}: {err}",
                            record.id
                        );
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

    // Sidecar image drift remediation. Every blueprint calls
    // reconcile_on_startup() at boot, so folding this here means the cascade off
    // the manager's binary upgrade is universal: when the manager swaps the
    // operator binary (on-chain BinaryVersion CD loop), the new binary boots,
    // reconciles, and rolls any sandbox still on a stale image onto the current
    // SIDECAR_IMAGE — preserving each sandbox's secrets/identity. Without this,
    // sandboxes stay pinned to their birth image forever (e.g. one that predates
    // opencode → every agent run fails). Drift detection ⇒ no-op when unchanged.
    // Policy via SIDECAR_UPGRADE_POLICY (default Auto; `manual` only reports).
    match crate::runtime::reconcile_sidecar_images(
        crate::runtime::SidecarUpgradePolicy::from_env(),
        None,
    )
    .await
    {
        Ok(report)
            if !report.upgraded.is_empty()
                || !report.failed.is_empty()
                || !report.pending.is_empty() =>
        {
            tracing::info!(
                target_image = %report.target_image,
                upgraded = report.upgraded.len(),
                failed = report.failed.len(),
                pending = report.pending.len(),
                "reconcile: sidecar image drift remediation complete"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("reconcile: sidecar image reconcile failed: {e}"),
    }
}
