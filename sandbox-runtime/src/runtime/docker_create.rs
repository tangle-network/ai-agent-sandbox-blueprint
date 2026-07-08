use super::*;

pub(crate) async fn create_sidecar_docker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let previous_store_entry = existing_store_entry_for_override(&sandbox_id)?;

    // Recreating an existing sandbox reuses its existing store slot.
    enforce_sandbox_count_limit(config, previous_store_entry.is_some())?;

    let builder = docker_builder().await?;

    // Use the user-supplied image if provided, otherwise fall back to the
    // operator's SIDECAR_IMAGE env var.
    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    ensure_image_pulled(&builder, &effective_image).await?;
    let original_image = effective_image.clone();

    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };
    let container_name = format!("sidecar-{sandbox_id}");

    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let env_vars = build_env_vars(
        &effective_env,
        &token,
        config.container_port,
        &request.capabilities_json,
    )?;

    let metadata = parse_json_object(&request.metadata_json, "metadata_json")?;
    // Extract snapshot_destination before metadata is consumed by merge/labels
    let snapshot_destination = metadata
        .as_ref()
        .and_then(|v| v.get("snapshot_destination"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let metadata = merge_metadata(metadata, &request.image, &request.stack)?;
    let labels = match metadata {
        Some(Value::Object(map)) => Some(
            map.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
                .collect(),
        ),
        _ => None,
    };

    // Parse extra ports from metadata_json (e.g. {"ports": [3000, 8080]}).
    let extra_ports = parse_extra_ports(&request.metadata_json, &request.port_mappings);

    let override_config = build_docker_config(
        config,
        request.ssh_enabled,
        request.cpu_cores,
        request.memory_mb,
        labels,
        &extra_ports,
    );

    let mut container = Container::new(builder.client(), effective_image)
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let extra_port_seed = extra_ports
            .iter()
            .copied()
            .map(|port| (port, 0u16))
            .collect::<HashMap<_, _>>();
        let (sidecar_url, sidecar_port, ssh_port, extra_port_map) =
            retry_port_mapping_lookup_inner(
                "create endpoint resolution",
                &container_id,
                PORT_MAPPING_RETRY_ATTEMPTS,
                PORT_MAPPING_RETRY_DELAY_MS,
                || {
                    refresh_port_mapping(
                        builder.client(),
                        &container_id,
                        config.container_port,
                        request.ssh_enabled,
                        &config.public_host,
                        &extra_port_seed,
                    )
                },
            )
            .await?;

        // Repair workspace ownership before the sidecar spawns OpenCode as the
        // agent user (uid 1000).  Without this, /home/agent dirs may be root-owned
        // and the demoted process crashes with EACCES on mkdir .local.
        match docker_exec_as_user(
            &container_id,
            "root",
            "chown -R agent:agent /home/agent 2>/dev/null || true",
        )
        .await
        {
            Ok(r) if r.exit_code != 0 => {
                tracing::warn!(
                    sandbox_id,
                    exit_code = r.exit_code,
                    stderr = %r.stderr,
                    "workspace ownership repair returned non-zero (continuing)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    sandbox_id,
                    error = %e,
                    "workspace ownership repair failed (continuing)"
                );
            }
            _ => {}
        }

        // Pre-create directories that the sidecar's root process will try to
        // mkdir before demoting to uid 1000.  Without DAC_OVERRIDE the root
        // process cannot write to agent-owned /home/agent, so we create them
        // as the agent user who legitimately owns the parent directory.
        match docker_exec_as_user(
            &container_id,
            "agent",
            "mkdir -p /home/agent/.opencode-home/.config",
        )
        .await
        {
            Ok(r) if r.exit_code != 0 => {
                tracing::warn!(
                    sandbox_id,
                    exit_code = r.exit_code,
                    stderr = %r.stderr,
                    "opencode-home pre-creation returned non-zero (continuing)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    sandbox_id,
                    error = %e,
                    "opencode-home pre-creation failed (continuing)"
                );
            }
            _ => {}
        }

        let now = crate::util::now_ts();
        let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
        let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

        let record = SandboxRecord {
            id: sandbox_id.clone(),
            container_id: container_id.clone(),
            sidecar_url,
            sidecar_port,
            ssh_port,
            token,
            created_at: now,
            cpu_cores: request.cpu_cores,
            memory_mb: request.memory_mb,
            state: SandboxState::Running,
            idle_timeout_seconds: idle_timeout,
            max_lifetime_seconds: max_lifetime,
            last_activity_at: now,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image,
            base_env_json: request.env_json.clone(),
            user_env_json: request.user_env_json.clone(),
            snapshot_destination,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: request.name.clone(),
            agent_identifier: request.agent_identifier.clone(),
            metadata_json: request.metadata_json.clone(),
            disk_gb: request.disk_gb,
            stack: request.stack.clone(),
            owner: request.owner.clone(),
            service_id: request.service_id,
            tee_config: None,
            extra_ports: extra_port_map,
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
            capabilities_json: request.capabilities_json.clone(),
        };

        let mut sealed = record.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(sandbox_id.clone(), sealed)?;

        let ready_record = if request.ssh_enabled {
            ensure_ssh_ready(&record).await?
        } else {
            record.clone()
        };

        crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

        Ok(ready_record)
    }
    .await;

    if finish.is_err() {
        let _ = restore_previous_store_entry(&sandbox_id, previous_store_entry);
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}
