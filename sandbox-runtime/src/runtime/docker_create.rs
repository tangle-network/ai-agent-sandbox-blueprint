use super::*;

fn workspace_bootstrap_commands(workspace_root: &str) -> (String, String) {
    let config_dir = format!("{workspace_root}/.opencode-home/.config");
    let escaped_root = crate::util::shell_escape(workspace_root);
    let escaped_config = crate::util::shell_escape(&config_dir);
    let agent_mkdir = crate::util::shell_escape(&format!("mkdir -p {escaped_config}"));
    let root = format!(
        "mkdir -p {escaped_config} 2>/dev/null \
         || su agent -s /bin/sh -c {agent_mkdir} 2>/dev/null; \
         chown -R agent:agent {escaped_root} 2>/dev/null || true; \
         test -d {escaped_config}"
    );
    let agent =
        format!("mkdir -p {escaped_config} && test -w {escaped_root} && test -w {escaped_config}");
    (root, agent)
}

/// Create the requested workspace and prove the sandbox user can write it.
///
/// The root exec repairs images that ship root-owned directories. If root
/// cannot create inside an agent-owned home, Docker runs the same command as
/// the agent user. A failed write check aborts sandbox creation.
pub(crate) async fn run_workspace_bootstrap(
    exec_client: &docktopus::bollard::Docker,
    container_id: &str,
    sandbox_id: &str,
    workspace_root: &str,
) -> Result<()> {
    let (root_command, agent_command) = workspace_bootstrap_commands(workspace_root);
    let bootstrap_verified = match docker_exec_as_user_with_client(
        exec_client,
        container_id,
        "root",
        &root_command,
    )
    .await
    {
        Ok(r) if r.exit_code == 0 => true,
        Ok(r) => {
            tracing::info!(
                sandbox_id,
                exit_code = r.exit_code,
                stderr = %r.stderr,
                "merged workspace bootstrap could not verify dirs; falling back to agent-user mkdir"
            );
            false
        }
        Err(e) => {
            tracing::warn!(
                sandbox_id,
                error = %e,
                "merged workspace bootstrap failed; falling back to agent-user mkdir"
            );
            false
        }
    };
    if !bootstrap_verified {
        let result =
            docker_exec_as_user_with_client(exec_client, container_id, "agent", &agent_command)
                .await?;
        if result.exit_code != 0 {
            return Err(SandboxError::Docker(format!(
                "workspace bootstrap failed for {sandbox_id} at {workspace_root}: {}",
                result.stderr
            )));
        }
        return Ok(());
    }

    let verification = docker_exec_as_user_with_client(
        exec_client,
        container_id,
        "agent",
        &format!(
            "test -w {} && test -w {}",
            crate::util::shell_escape(workspace_root),
            crate::util::shell_escape(&format!("{workspace_root}/.opencode-home/.config"))
        ),
    )
    .await?;
    if verification.exit_code != 0 {
        return Err(SandboxError::Docker(format!(
            "workspace is not writable for {sandbox_id} at {workspace_root}: {}",
            verification.stderr
        )));
    }
    Ok(())
}

/// Docker-backed create: try the warm pool first, then cold.
///
/// The warm claim only applies to a fresh create — both `token_override` and
/// `sandbox_id_override` are `None`. Recreate, image-upgrade, and
/// snapshot-restore paths pass an override (they need a specific token/id and
/// their own store-rollback semantics) and go straight to
/// [`cold_create_sidecar_docker`].
pub(crate) async fn create_sidecar_docker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, CreateTimings)> {
    if token_override.is_none() && sandbox_id_override.is_none() {
        let sandbox_id = next_sandbox_id();
        let claim_stage = std::time::Instant::now();
        let outcome = crate::docker_warm::claim_docker_warm(request, &sandbox_id).await?;
        let warm_claim_elapsed = claim_stage.elapsed();
        match outcome {
            crate::docker_warm::DockerWarmOutcome::Claimed(claim) => {
                return finish_warm_claim_docker(request, claim, sandbox_id, warm_claim_elapsed)
                    .await;
            }
            crate::docker_warm::DockerWarmOutcome::Miss(miss) => {
                tracing::debug!(
                    sandbox_id = %sandbox_id,
                    reason = %miss,
                    "docker warm-pool miss; falling back to cold create"
                );
            }
        }
    }
    cold_create_sidecar_docker(request, token_override, sandbox_id_override).await
}

/// Finish a warm claim: build the store record binding all per-request state
/// (owner/name/agent_identifier/service_id/metadata/timeouts) onto the reused
/// warm container id + baked warm token + resolved endpoint, insert it Running,
/// and record metrics. Mirrors the record-build + insert tail of
/// [`cold_create_sidecar_docker`]. The token MUST be the baked warm token
/// (`claim.token`): it is inside the container's immutable env, so a fresh token
/// would not match what the sidecar authenticates against.
async fn finish_warm_claim_docker(
    request: &CreateSandboxParams,
    claim: crate::docker_warm::DockerWarmClaim,
    sandbox_id: String,
    warm_claim_elapsed: std::time::Duration,
) -> Result<(SandboxRecord, CreateTimings)> {
    let config = SidecarRuntimeConfig::load();
    let mut timings = CreateTimings {
        warm_claim: Some(warm_claim_elapsed),
        ..Default::default()
    };

    // Shape gate guarantees the request's image equals the pooled image; resolve
    // it the same way the cold path does for `original_image`.
    let original_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    let metadata = parse_json_object(&request.metadata_json, "metadata_json")?;
    let snapshot_destination = metadata
        .as_ref()
        .and_then(|v| v.get("snapshot_destination"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now = crate::util::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);
    let container_id = claim.container_id.clone();

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: claim.container_id,
        sidecar_url: claim.sidecar_url,
        sidecar_port: claim.sidecar_port,
        ssh_port: claim.ssh_port,
        token: claim.token,
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
        extra_ports: claim.extra_ports,
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: request.capabilities_json.clone(),
    };

    let insert = async {
        let stage = std::time::Instant::now();
        let mut sealed = record.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(sandbox_id.clone(), sealed)?;
        timings.store_insert = Some(stage.elapsed());
        crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);
        Ok::<SandboxRecord, SandboxError>(record.clone())
    }
    .await;

    match insert {
        Ok(ready_record) => Ok((ready_record, timings)),
        Err(err) => {
            // The container was already renamed onto sandbox_id and cannot
            // return to the pool. It still carries the warm label, so the next
            // restart reconcile would reap it, but reap now to avoid holding
            // RAM + a host port until then.
            if let Ok(builder) = docker_builder().await {
                cleanup_orphaned_container(&builder, &container_id).await;
            }
            Err(err)
        }
    }
}

/// Docker-backed cold create with per-stage [`CreateTimings`]. The shared entry
/// point (`create_sidecar_with_token`) fills the permit/admission/total fields;
/// this function fills every Docker stage it passes through.
pub(crate) async fn cold_create_sidecar_docker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, CreateTimings)> {
    let mut timings = CreateTimings::default();
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    // Count cap + memory budget were already enforced in a single store pass
    // by `admit_sandbox_resources` under the CREATION_PERMIT (still held); the
    // slot-reuse decision now lives entirely in that scan (keyed off the
    // override id). This entry is read solely to restore the prior record on a
    // create failure (`restore_previous_store_entry`), so the Docker rollback
    // path can't clobber the sandbox it replaced.
    let previous_store_entry = existing_store_entry_for_override(&sandbox_id)?;

    let stage = std::time::Instant::now();
    let builder = docker_builder().await?;
    timings.docker_connect = Some(stage.elapsed());

    // Use the user-supplied image if provided, otherwise fall back to the
    // operator's SIDECAR_IMAGE env var.
    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    let stage = std::time::Instant::now();
    ensure_image_pulled(&builder, &effective_image).await?;
    timings.image_pull = Some(stage.elapsed());
    let original_image = effective_image.clone();

    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };
    let container_name = format!("sidecar-{sandbox_id}");

    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let container_environment = build_container_environment(
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
        .env(container_environment.vars)
        .config_override(override_config);

    // Split Docker-side create from start so each hop is visible. On a
    // transient create failure we do NOT bail: `Container::start(false)`
    // re-runs create while the container id is unset, so the pre-existing
    // retry-once semantics of `start_container_with_retry` are preserved.
    let stage = std::time::Instant::now();
    if let Err(err) = docker_timeout("create_container", container.create()).await {
        tracing::debug!(error = %err, "container create failed; start path will retry it");
    }
    timings.container_create = Some(stage.elapsed());

    let stage = std::time::Instant::now();
    start_container_with_retry(&mut container).await?;
    timings.container_start = Some(stage.elapsed());

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
        let stage = std::time::Instant::now();
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
        timings.port_mapping = Some(stage.elapsed());

        let stage = std::time::Instant::now();
        // Workspace bootstrap (chown + pre-create ~/.opencode-home) on the
        // already-connected client — see `run_workspace_bootstrap`.
        run_workspace_bootstrap(
            &builder.client(),
            &container_id,
            &sandbox_id,
            &container_environment.workspace_root,
        )
        .await?;
        timings.bootstrap_exec = Some(stage.elapsed());

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

        let stage = std::time::Instant::now();
        let mut sealed = record.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(sandbox_id.clone(), sealed)?;
        timings.store_insert = Some(stage.elapsed());

        let ready_record = if request.ssh_enabled {
            let stage = std::time::Instant::now();
            let ready = ensure_ssh_ready(&record).await?;
            timings.ssh_ready = Some(stage.elapsed());
            ready
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
    finish.map(|record| (record, timings))
}

#[cfg(test)]
mod workspace_bootstrap_tests {
    use super::workspace_bootstrap_commands;

    #[test]
    fn commands_target_requested_workspace() {
        let (root, agent) = workspace_bootstrap_commands("/home/agent/vault");
        for command in [&root, &agent] {
            assert!(command.contains("'/home/agent/vault'"));
            assert!(command.contains("'/home/agent/vault/.opencode-home/.config'"));
        }
        assert!(!root.contains("chown -R agent:agent '/home/agent'"));
    }

    #[test]
    fn commands_quote_workspace_components() {
        let (root, agent) = workspace_bootstrap_commands("/home/agent/team's vault");
        for command in [root, agent] {
            assert!(command.contains("team'\"'\"'s vault"));
        }
    }
}
