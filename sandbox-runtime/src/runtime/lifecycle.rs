use super::*;

/// Stop a running sandbox container, updating its state to `Stopped`.
///
/// For TEE-managed sandboxes, delegates to the TEE backend's `stop()` method.
/// For standard Docker sandboxes, stops via the Docker API directly.
pub async fn stop_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Stopped {
        return Err(SandboxError::Validation(
            "Sandbox is already stopped".into(),
        ));
    }

    // TEE-managed sandbox: delegate to the TEE backend.
    if let Some(deployment_id) = &record.tee_deployment_id
        && let Some(backend) = crate::tee::try_tee_backend()
    {
        backend.stop(deployment_id).await?;
        let now = crate::util::now_ts();
        let _ = sandboxes()?.update(&record.id, |r| {
            r.state = SandboxState::Stopped;
            r.stopped_at = Some(now);
        });
        return Ok(());
    }

    if record_uses_firecracker(record) {
        crate::firecracker::stop(&record.container_id).await?;
        let now = crate::util::now_ts();
        let _ = sandboxes()?.update(&record.id, |r| {
            r.state = SandboxState::Stopped;
            r.stopped_at = Some(now);
        });
        return Ok(());
    }

    // Standard Docker path.
    let builder = docker_builder().await?;
    let mut container = docker_timeout(
        "load_container",
        Container::from_id(builder.client(), &record.container_id),
    )
    .await?;
    docker_timeout("stop_container", container.stop()).await?;

    let now = crate::util::now_ts();
    let _ = sandboxes()?.update(&record.id, |r| {
        r.state = SandboxState::Stopped;
        r.stopped_at = Some(now);
    });
    Ok(())
}

/// Poll a sidecar's `/health` endpoint until it responds successfully or the timeout expires.
pub(crate) async fn wait_for_sidecar_health(sidecar_url: &str, timeout_secs: u64) -> bool {
    let ready = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        loop {
            let url = format!("{sidecar_url}/health");
            if let Ok(resp) = crate::util::http_client().map(|c| c.get(&url))
                && let Ok(r) = resp.send().await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;
    ready.is_ok()
}

/// Re-inspect a running Docker-backed sandbox and persist its current host port mappings.
///
/// This is the authoritative recovery path for stale localhost port bindings
/// after Docker restart/start operations.
pub async fn refresh_docker_sandbox_endpoint(record: &SandboxRecord) -> Result<SandboxRecord> {
    if !supports_docker_endpoint_refresh(record) {
        return Err(SandboxError::Validation(format!(
            "Sandbox {} does not use Docker-backed dynamic port refresh",
            record.id
        )));
    }

    let builder = docker_builder().await?;
    let config = SidecarRuntimeConfig::load();
    let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
        "refresh endpoint resolution",
        builder.client(),
        &record.container_id,
        config.container_port,
        record.ssh_port.is_some(),
        &config.public_host,
        &record.extra_ports,
    )
    .await?;

    let updated = sandboxes()?.update(&record.id, |r| {
        r.sidecar_url = sidecar_url.clone();
        r.sidecar_port = sidecar_port;
        r.ssh_port = ssh_port;
        r.extra_ports = extra_ports.clone();
    })?;

    if !updated {
        return Err(SandboxError::NotFound(format!(
            "Sandbox '{}' not found while refreshing endpoint",
            record.id
        )));
    }

    get_sandbox_by_id(&record.id)
}

pub(crate) async fn stop_started_container(
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
) -> Result<()> {
    let mut container =
        docker_timeout("load_container", Container::from_id(client, container_id)).await?;
    docker_timeout("stop_container", container.stop()).await?;
    Ok(())
}

/// Resume a stopped sandbox, restoring from container, snapshot image, or S3 as available.
pub async fn resume_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Running {
        return Err(SandboxError::Validation(
            "Sandbox is already running".into(),
        ));
    }
    if record_uses_firecracker(record) {
        // Re-start the VM via the in-process driver. The primitive can flip
        // a stopped VM back to running but cannot yet expose a host-reachable
        // endpoint, so we leave the persisted `sidecar_url` untouched. If the
        // future driver release returns an endpoint, plumb it through here.
        let resumed = crate::firecracker::start(&record.container_id).await?;
        if let Some(sidecar_url) = resumed.endpoint {
            let sidecar_port =
                parse_url_port(&sidecar_url).unwrap_or(SidecarRuntimeConfig::load().container_port);
            if !wait_for_sidecar_health(&sidecar_url, 30).await {
                let _ = crate::firecracker::stop(&record.container_id).await;
                return Err(SandboxError::Unavailable(format!(
                    "Resume failed: firecracker sidecar for sandbox {} did not become healthy",
                    record.id
                )));
            }
            let now = crate::util::now_ts();
            let _ = sandboxes()?.update(&record.id, |r| {
                r.state = SandboxState::Running;
                r.stopped_at = None;
                r.last_activity_at = now;
                r.sidecar_url = sidecar_url.clone();
                r.sidecar_port = sidecar_port;
            });
            return Ok(());
        }
        // No endpoint plumbing yet: roll the VM back and report loudly.
        let _ = crate::firecracker::stop(&record.container_id).await;
        return Err(SandboxError::Unsupported(format!(
            "resume for runtime_backend=firecracker is not wired end-to-end yet: the in-process \
             driver started sandbox {} but cannot expose a host-reachable sidecar endpoint until \
             microvm-runtime 0.2.0 ships network setup",
            record.id
        )));
    }

    // For TEE-managed sandboxes, tee_deployment_id holds the real Docker container
    // ID (Direct backend) or cloud deployment ID (cloud backends). Use it for
    // Docker operations when available so the `tee-` prefixed container_id is
    // bypassed.
    let effective_container_id = record
        .tee_deployment_id
        .as_deref()
        .unwrap_or(&record.container_id);

    // Tier 1 (Hot): container still exists -> docker start
    if record.container_removed_at.is_none() {
        let builder = docker_builder().await?;
        let try_start = async {
            let mut container = docker_timeout(
                "load_container",
                Container::from_id(builder.client(), effective_container_id),
            )
            .await?;
            start_container_with_retry(&mut container).await?;
            Ok::<(), SandboxError>(())
        };
        match try_start.await {
            Ok(()) => {
                let (resumed_record, sidecar_ready) = match refresh_docker_sandbox_endpoint(record)
                    .await
                {
                    Ok(updated) => (updated, false),
                    Err(err) => {
                        blueprint_sdk::info!(
                            "resume: could not refresh port mapping for sandbox {}: {err}",
                            record.id
                        );
                        if wait_for_sidecar_health(&record.sidecar_url, 30).await {
                            blueprint_sdk::info!(
                                "resume: using stored sidecar URL for sandbox {} after refresh failure",
                                record.id
                            );
                            (record.clone(), true)
                        } else {
                            let _ =
                                stop_started_container(builder.client(), effective_container_id)
                                    .await;
                            return Err(SandboxError::Unavailable(format!(
                                "Resume failed: could not refresh sidecar URL for sandbox {}",
                                record.id
                            )));
                        }
                    }
                };

                if !sidecar_ready && !wait_for_sidecar_health(&resumed_record.sidecar_url, 30).await
                {
                    let _ = stop_started_container(builder.client(), effective_container_id).await;
                    return Err(SandboxError::Unavailable(format!(
                        "Resume failed: sidecar for sandbox {} did not become healthy at {}",
                        record.id, resumed_record.sidecar_url
                    )));
                }

                if resumed_record.ssh_port.is_some() {
                    let _ = restore_ssh_access(&resumed_record).await?;
                }

                let now = crate::util::now_ts();
                let _ = sandboxes()?.update(&record.id, |r| {
                    r.state = SandboxState::Running;
                    r.stopped_at = None;
                    r.last_activity_at = now;
                });
                return Ok(());
            }
            Err(err) => {
                blueprint_sdk::info!(
                    "resume: hot start failed for sandbox {}, trying warm: {err}",
                    record.id
                );
            }
        }
    }

    // Tier 2 (Warm): container gone, snapshot image exists -> create from image
    if record.snapshot_image_id.is_some() {
        create_from_snapshot_image(record).await?;
        return Ok(());
    }

    // Tier 3 (Cold): no image, S3 snapshot exists -> create from base + restore
    if record.snapshot_s3_url.is_some() {
        create_and_restore_from_s3(record).await?;
        return Ok(());
    }

    // Nothing available
    Err(SandboxError::Docker(format!(
        "Cannot resume sandbox {}: no container, snapshot image, or S3 snapshot available",
        record.id
    )))
}

/// Permanently destroy a sandbox, removing the container, image, and store entry.
///
/// For TEE-managed sandboxes, delegates to the TEE backend's `destroy()` method.
/// Accepts an explicit backend reference, or falls back to the global TEE backend.
pub async fn delete_sidecar(
    record: &SandboxRecord,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<()> {
    // If this is a TEE-managed sandbox, delegate to the backend.
    if let Some(deployment_id) = &record.tee_deployment_id {
        // Use explicit backend if provided, otherwise fall back to global.
        let backend = tee.map(Ok).unwrap_or_else(|| {
            crate::tee::try_tee_backend()
                .map(|b| b.as_ref())
                .ok_or_else(|| {
                    SandboxError::Validation(
                        "TEE sandbox has no backend available for deletion".into(),
                    )
                })
        })?;
        backend.destroy(deployment_id).await?;
        crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);
        return Ok(());
    }
    if record_uses_firecracker(record) {
        crate::firecracker::delete(&record.container_id).await?;
        crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);
        return Ok(());
    }
    // Default Docker removal path.
    delete_sidecar_docker(record).await
}

pub(crate) async fn delete_sidecar_docker(record: &SandboxRecord) -> Result<()> {
    let builder = docker_builder().await?;
    let container = docker_timeout(
        "load_container",
        Container::from_id(builder.client(), &record.container_id),
    )
    .await?;
    docker_timeout(
        "remove_container",
        container.remove(Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        })),
    )
    .await?;

    crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);

    Ok(())
}
