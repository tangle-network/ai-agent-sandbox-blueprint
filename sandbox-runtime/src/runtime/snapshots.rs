use super::*;

/// Docker-commit a stopped container to preserve filesystem state. Returns the image ID.
pub async fn commit_container(record: &SandboxRecord) -> Result<String> {
    if record_uses_firecracker(record) {
        return Err(SandboxError::Validation(
            "Snapshot image commit is not supported for runtime_backend=firecracker".into(),
        ));
    }
    let builder = docker_builder().await?;
    use docktopus::bollard::image::CommitContainerOptions;
    let options = CommitContainerOptions {
        container: record.container_id.clone(),
        repo: format!("sandbox-snapshot/{}", record.id),
        tag: "latest".to_string(),
        comment: format!("Auto-snapshot of sandbox {}", record.id),
        pause: true,
        ..Default::default()
    };
    let repo_tag = format!("sandbox-snapshot/{}:latest", record.id);
    let response = docker_timeout(
        "commit_container",
        builder
            .client()
            .commit_container(options, BollardConfig::<String>::default()),
    )
    .await?;
    Ok(response.id.filter(|s| !s.is_empty()).unwrap_or(repo_tag))
}

/// Remove a committed snapshot image from the local Docker daemon.
pub async fn remove_snapshot_image(image_id: &str) -> Result<()> {
    let builder = docker_builder().await?;
    docker_timeout(
        "remove_image",
        builder.client().remove_image(image_id, None, None),
    )
    .await?;
    Ok(())
}

/// Create a new container from a previously committed Docker image.
pub async fn create_from_snapshot_image(record: &SandboxRecord) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    let image_id = record
        .snapshot_image_id
        .as_deref()
        .ok_or_else(|| SandboxError::Docker("No snapshot image available".into()))?;

    let ssh_enabled = record.ssh_port.is_some();
    let effective_env = record.effective_env_json();
    let env_vars = build_env_vars(
        &effective_env,
        &record.token,
        config.container_port,
        &record.capabilities_json,
    )?;
    let ep: Vec<u16> = record.extra_ports.keys().copied().collect();
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
        &ep,
    );

    let container_name = format!("sidecar-{}-warm", record.id);
    let mut container = Container::new(builder.client(), image_id.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
            "warm restore endpoint resolution",
            builder.client(),
            &container_id,
            config.container_port,
            ssh_enabled,
            &config.public_host,
            &record.extra_ports,
        )
        .await?;

        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            return Err(SandboxError::Unavailable(format!(
                "Resume failed: warm sidecar for sandbox {} did not become healthy at {}",
                record.id, sidecar_url
            )));
        }

        let now = crate::util::now_ts();
        let mut updated = record.clone();
        updated.container_id = container_id.clone();
        updated.sidecar_url = sidecar_url;
        updated.sidecar_port = sidecar_port;
        updated.ssh_port = ssh_port;
        updated.state = SandboxState::Running;
        updated.stopped_at = None;
        updated.last_activity_at = now;
        updated.container_removed_at = None;
        updated.snapshot_image_id = None;
        updated.extra_ports = extra_ports;

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        if ssh_enabled {
            restore_ssh_access(&updated).await
        } else {
            Ok(updated)
        }
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}

/// Create a fresh container from the original base image, then restore workspace from S3 snapshot.
pub async fn create_and_restore_from_s3(record: &SandboxRecord) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    let s3_url = record
        .snapshot_s3_url
        .as_deref()
        .ok_or_else(|| SandboxError::Docker("No S3 snapshot URL available".into()))?;

    let image = if record.original_image.is_empty() {
        &config.image
    } else {
        &record.original_image
    };

    ensure_image_pulled(&builder, image).await?;

    let ssh_enabled = record.ssh_port.is_some();
    let effective_env = record.effective_env_json();
    let env_vars = build_env_vars(
        &effective_env,
        &record.token,
        config.container_port,
        &record.capabilities_json,
    )?;
    let ep: Vec<u16> = record.extra_ports.keys().copied().collect();
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
        &ep,
    );

    let container_name = format!("sidecar-{}-cold", record.id);
    let mut container = Container::new(builder.client(), image.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
            "cold restore endpoint resolution",
            builder.client(),
            &container_id,
            config.container_port,
            ssh_enabled,
            &config.public_host,
            &record.extra_ports,
        )
        .await?;
        let token = &record.token;

        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            return Err(SandboxError::Unavailable(format!(
                "Resume failed: cold sidecar for sandbox {} did not become healthy at {}",
                record.id, sidecar_url
            )));
        }

        // Restore workspace from S3 snapshot
        let restore_cmd = format!(
            "set -euo pipefail; curl -fsSL {} | tar -xzf - -C /",
            crate::util::shell_escape(s3_url)
        );
        let payload = serde_json::json!({
            "command": format!("sh -c {}", crate::util::shell_escape(&restore_cmd)),
        });
        if let Err(err) =
            crate::http::sidecar_post_json(&sidecar_url, "/terminals/commands", token, payload)
                .await
        {
            blueprint_sdk::error!("S3 restore failed for sandbox {}: {err}", record.id);
            return Err(SandboxError::Docker(format!("S3 restore failed: {err}")));
        }

        let now = crate::util::now_ts();
        let mut updated = record.clone();
        updated.container_id = container_id.clone();
        updated.sidecar_url = sidecar_url;
        updated.sidecar_port = sidecar_port;
        updated.ssh_port = ssh_port;
        updated.state = SandboxState::Running;
        updated.stopped_at = None;
        updated.last_activity_at = now;
        updated.container_removed_at = None;
        updated.image_removed_at = None;
        updated.extra_ports = extra_ports;
        updated.snapshot_s3_url = None;

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        if ssh_enabled {
            restore_ssh_access(&updated).await
        } else {
            Ok(updated)
        }
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}
