use super::*;

pub(crate) async fn create_sidecar_firecracker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let previous_store_entry = existing_store_entry_for_override(&sandbox_id)?;

    enforce_sandbox_count_limit(config, previous_store_entry.is_some())?;

    // Parse and validate port mappings strictly — malformed entries fail
    // fast here rather than being silently dropped. Both legacy `[3000]` and
    // structured `[{container_port,host_port,protocol}]` shapes are accepted.
    // The parsed entries are persisted on the record and wired into the
    // per-VM iptables PREROUTING DNAT chain by `firecracker::create_and_start`.
    let metadata_value =
        parse_json_object(&request.metadata_json, "metadata_json")?.unwrap_or(Value::Null);
    let parsed_ports = parse_metadata_ports(&metadata_value)?;

    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    let metadata_raw = parse_json_object(&request.metadata_json, "metadata_json")?;
    let snapshot_destination = metadata_raw
        .as_ref()
        .and_then(|v| v.get("snapshot_destination"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let metadata = merge_metadata(metadata_raw, &request.image, &request.stack)?;
    let labels = match metadata {
        Some(Value::Object(map)) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
            .collect::<HashMap<String, String>>(),
        _ => HashMap::new(),
    };

    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let mut env = HashMap::new();
    env.insert(
        "SIDECAR_PORT".to_string(),
        config.container_port.to_string(),
    );
    if let Some(caps) = parse_sidecar_capabilities(&request.capabilities_json) {
        env.insert("SIDECAR_CAPABILITIES".to_string(), caps);
    }
    if !effective_env.trim().is_empty()
        && let Some(Value::Object(map)) = parse_json_object(&effective_env, "env_json")?
    {
        for (key, value) in map {
            let val = match value {
                Value::String(v) => v,
                Value::Number(v) => v.to_string(),
                Value::Bool(v) => v.to_string(),
                _ => continue,
            };
            env.insert(key, val);
        }
    }

    // Forward the operator's Tangle Intelligence telemetry config into every
    // sidecar so the agent-runtime loop OTEL (token usage, decisions, tool
    // calls, eval scores — the self-improvement signal) streams to Intelligence
    // via the sidecar's sdk-telemetry sink. The operator process carries
    // TANGLE_API_KEY (sk-tan-*) for its own OTLP export; the sidecar runs the
    // agent loops, so it needs the same credential. Gated on the key being
    // present; explicit per-sandbox TELEMETRY_*/OTEL_* env always wins (we only
    // fill what isn't already set).
    inject_intelligence_telemetry_env(&mut env);

    let create_request = crate::firecracker::FirecrackerCreateRequest {
        session_id: sandbox_id.clone(),
        image: effective_image.clone(),
        env,
        labels,
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
        disk_gb: request.disk_gb,
        ports: parsed_ports.clone(),
    };

    let provisioned = crate::firecracker::create_and_start(create_request).await?;
    let sidecar_url = provisioned.container.endpoint.ok_or_else(|| {
        // `create_and_start` always populates `endpoint` once the VM is
        // reachable; an absent value here means the primitive shape changed
        // in a way we cannot route around. Surface it explicitly rather than
        // silently mis-routing into a sandbox with no URL.
        SandboxError::Unavailable(format!(
            "firecracker driver started sandbox {sandbox_id}, but did not return an endpoint"
        ))
    })?;

    let generated_token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };
    let token = provisioned.sidecar_auth_token.unwrap_or(generated_token);
    let metadata_json =
        metadata_with_runtime_backend(&request.metadata_json, RuntimeBackend::Firecracker)?;
    let sidecar_port = parse_url_port(&sidecar_url).unwrap_or(config.container_port);

    let now = crate::util::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: provisioned.container.id,
        sidecar_url,
        sidecar_port,
        ssh_port: None,
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
        original_image: effective_image,
        base_env_json: request.env_json.clone(),
        user_env_json: request.user_env_json.clone(),
        snapshot_destination,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: request.name.clone(),
        agent_identifier: request.agent_identifier.clone(),
        metadata_json,
        disk_gb: request.disk_gb,
        stack: request.stack.clone(),
        owner: request.owner.clone(),
        service_id: request.service_id,
        tee_config: None,
        // Persist the parsed structured port mappings on the record so they
        // survive restart and so callers reading the sandbox can introspect
        // intended forwarding before microvm-runtime ships the network
        // layer that actually routes them. Empty when no `metadata.ports`
        // were requested.
        extra_ports: parsed_ports
            .iter()
            .map(|p| (p.container_port, p.host_port))
            .collect(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: request.capabilities_json.clone(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed)?;
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok(record)
}
