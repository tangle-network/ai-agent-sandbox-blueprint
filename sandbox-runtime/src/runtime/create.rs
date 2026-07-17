use super::*;

/// Create a new sandbox container.
///
/// `token_override`: when `Some`, uses the given token instead of generating
/// a new one. Used by `recreate_sidecar_with_env` to preserve the original
/// token across container re-creation.
pub async fn create_sidecar(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    create_sidecar_with_token(request, tee, None, None)
        .await
        .map(|(record, attestation, _timings)| (record, attestation))
}

/// [`create_sidecar`] plus the measured per-stage [`CreateTimings`] breakdown.
///
/// Same create, same side effects — the timings are pure observation. Used by
/// the lifecycle bench (`tests/lifecycle_bench.rs`) and available to any
/// caller that wants the real create latency decomposition.
pub async fn create_sidecar_timed(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<(
    SandboxRecord,
    Option<crate::tee::AttestationReport>,
    CreateTimings,
)> {
    create_sidecar_with_token(request, tee, None, None).await
}

/// Internal: create sidecar with optional token override.
///
/// Acquires [`CREATION_PERMIT`] to serialize the count-check + create
/// sequence and prevent TOCTOU races on the sandbox limit.
pub(crate) async fn create_sidecar_with_token(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(
    SandboxRecord,
    Option<crate::tee::AttestationReport>,
    CreateTimings,
)> {
    let requested = std::time::Instant::now();
    let _creation_permit = acquire_creation_permit().await;
    let permit_wait = requested.elapsed();
    // Resource admission runs under the permit and before backend dispatch:
    // per-sandbox maxima (reject over-max, clamp unlimited-to-max) and the
    // host memory budget apply identically to Docker, Firecracker, and TEE.
    let admission_span = std::time::Instant::now();
    let admitted =
        admit_sandbox_resources(SidecarRuntimeConfig::load(), request, sandbox_id_override)?;
    let admission = admission_span.elapsed();
    let request = &admitted;
    let backend = resolve_runtime_backend(request)?;
    let (record, attestation, mut timings) = match backend {
        RuntimeBackend::Tee => {
            let backend = tee.ok_or_else(|| {
                SandboxError::Validation(
                    "TEE runtime selected but no TEE backend configured".into(),
                )
            })?;
            validate_requested_tee_backend(request, backend)?;
            let (record, attestation) =
                create_sidecar_tee(request, backend, token_override, sandbox_id_override).await?;
            (record, attestation, CreateTimings::default())
        }
        RuntimeBackend::Firecracker => {
            let record =
                create_sidecar_firecracker(request, token_override, sandbox_id_override).await?;
            (record, None, CreateTimings::default())
        }
        RuntimeBackend::Docker => {
            let (record, timings) =
                create_sidecar_docker(request, token_override, sandbox_id_override).await?;
            (record, None, timings)
        }
    };
    timings.permit_wait = Some(permit_wait);
    timings.admission = Some(admission);
    timings.total = requested.elapsed();
    timings.log(&record.id, backend);
    Ok((record, attestation, timings))
}

pub(crate) fn validate_requested_tee_backend(
    request: &CreateSandboxParams,
    backend: &dyn crate::tee::TeeBackend,
) -> Result<()> {
    let Some(config) = request.tee_config.as_ref() else {
        return Ok(());
    };

    if let Some(nonce) = &config.attestation_nonce {
        crate::tee::validate_attestation_nonce(nonce)?;
        if !nonce.is_empty() && !backend.supports_attestation_report_data() {
            return Err(SandboxError::Validation(format!(
                "TEE backend {:?} does not support caller-supplied attestation nonces",
                backend.tee_type()
            )));
        }
    }

    if config.required
        && config.tee_type != crate::tee::TeeType::None
        && config.tee_type != backend.tee_type()
    {
        return Err(SandboxError::Validation(format!(
            "Requested TEE type {:?} is not available on configured backend {:?}",
            config.tee_type,
            backend.tee_type()
        )));
    }

    Ok(())
}

pub(crate) async fn create_sidecar_tee(
    request: &CreateSandboxParams,
    backend: &dyn crate::tee::TeeBackend,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    // Count cap + memory budget were already enforced for every backend —
    // TEE included — in a single store pass by `admit_sandbox_resources`,
    // under the [`CREATION_PERMIT`] acquired in `create_sidecar_with_token`
    // (still held here), so the check can't race. Unlike the Docker path,
    // the TEE path never used its previous store entry for rollback, so no
    // extra store read remains here.

    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };

    let extra_ports = parse_extra_ports(&request.metadata_json, &request.port_mappings);
    let mut tee_request = request.clone();
    tee_request.port_mappings = extra_ports;

    let tee_params = crate::tee::TeeDeployParams::from_sandbox_params(
        &sandbox_id,
        &tee_request,
        config.container_port,
        config.ssh_port,
        &token,
    );

    let deployment = backend.deploy(&tee_params).await?;

    let now = crate::util::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: format!("tee-{}", deployment.deployment_id),
        sidecar_url: deployment.sidecar_url,
        sidecar_port: config.container_port,
        ssh_port: deployment.ssh_port,
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
        original_image: request.image.clone(),
        base_env_json: request.env_json.clone(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: Some(deployment.deployment_id),
        tee_metadata_json: Some(deployment.metadata_json),
        tee_attestation_json: serde_json::to_string(&deployment.attestation).ok(),
        name: request.name.clone(),
        agent_identifier: request.agent_identifier.clone(),
        metadata_json: request.metadata_json.clone(),
        disk_gb: request.disk_gb,
        stack: request.stack.clone(),
        owner: request.owner.clone(),
        service_id: request.service_id,
        tee_config: request.tee_config.clone(),
        extra_ports: deployment.extra_ports,
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: request.capabilities_json.clone(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed)?;
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok((record, Some(deployment.attestation)))
}
