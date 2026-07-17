use super::*;

/// Merged post-start workspace bootstrap, run as root in ONE exec round-trip.
///
/// Covers both ownership states the image can ship in:
///  - root-owned `/home/agent` (repair path): root's own `mkdir` succeeds —
///    it owns the tree, no `DAC_OVERRIDE` needed — and must run BEFORE the
///    chown hands the tree to agent (after which `cap_drop=ALL` root can no
///    longer write into it);
///  - agent-owned `/home/agent` without the dirs (the canonical sidecar
///    image, verified: `agent 755`, no `.opencode-home`): root's mkdir is
///    denied, so drop to the agent user via `su` (the container keeps
///    SETUID/SETGID; the image ships /usr/bin/su) and create them as the
///    owner, matching the pre-merge dedicated agent exec.
///
/// The chown then runs unconditionally (`;` + `|| true`), exactly like the
/// pre-merge dedicated exec. The trailing `test -d` makes the exit code
/// report whether the dirs exist, so the caller knows to fall back to a
/// separate agent-user exec (images with an agent-owned tree but no `su`).
pub(crate) const WORKSPACE_BOOTSTRAP_ROOT_CMD: &str = "mkdir -p /home/agent/.opencode-home/.config 2>/dev/null \
     || su agent -s /bin/sh -c 'mkdir -p /home/agent/.opencode-home/.config' 2>/dev/null; \
     chown -R agent:agent /home/agent 2>/dev/null || true; \
     test -d /home/agent/.opencode-home/.config";

/// Last-resort fallback when the merged exec cannot produce the dirs (an
/// agent-owned `/home/agent` on an image without `su`): create them as the
/// agent user through Docker's own exec-user mechanism, which needs nothing
/// from the image — the pre-merge behavior, verbatim.
pub(crate) const WORKSPACE_BOOTSTRAP_AGENT_FALLBACK_CMD: &str =
    "mkdir -p /home/agent/.opencode-home/.config";

/// Docker-backed create with per-stage [`CreateTimings`]. The shared entry
/// point (`create_sidecar_with_token`) fills the permit/admission/total
/// fields; this function fills every Docker stage it passes through.
pub(crate) async fn create_sidecar_docker(
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
    // by `admit_sandbox_resources` under the CREATION_PERMIT (still held).
    // The previous entry is kept for slot-reuse semantics + failure rollback.
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
        // ── Workspace bootstrap ────────────────────────────────────────────
        // Two jobs, historically two exec round-trips (each also rebuilding a
        // Docker client — connect + ping — via `docker_exec_as_user`):
        //   1. as root:  chown -R agent:agent /home/agent — repair image
        //      builds that leave workspace dirs root-owned, so the sidecar's
        //      demoted (uid 1000) process doesn't crash with EACCES on
        //      mkdir .local;
        //   2. as agent: mkdir -p ~/.opencode-home/.config — pre-create dirs
        //      the sidecar's root process cannot create itself (cap_drop=ALL
        //      leaves root without DAC_OVERRIDE, so it cannot write into
        //      agent-owned directories).
        // Merged into ONE root exec on the already-connected `builder` client
        // (see WORKSPACE_BOOTSTRAP_ROOT_CMD for why mkdir-then-chown covers
        // the repair path). When its `test -d` verification fails — an image
        // that ships an agent-owned /home/agent without the dirs — fall back
        // to the pre-merge agent-user mkdir. Net round-trips: 1 on the repair
        // path and on images with the dirs baked in; 2 (unchanged) otherwise.
        // Failures stay warn-and-continue, exactly as before.
        let exec_client = builder.client();
        let bootstrap_verified = match docker_exec_as_user_with_client(
            &exec_client,
            &container_id,
            "root",
            WORKSPACE_BOOTSTRAP_ROOT_CMD,
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
            match docker_exec_as_user_with_client(
                &exec_client,
                &container_id,
                "agent",
                WORKSPACE_BOOTSTRAP_AGENT_FALLBACK_CMD,
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
        }
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
