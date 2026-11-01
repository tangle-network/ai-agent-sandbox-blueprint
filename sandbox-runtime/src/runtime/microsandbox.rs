use super::*;
use std::process::Command;

fn msb_bin() -> String {
    std::env::var("MSB_BIN").unwrap_or_else(|_| "msb".into())
}

fn run_msb(args: &[String]) -> Result<std::process::Output> {
    let bin = msb_bin();
    Command::new(&bin).args(args).output().map_err(|err| {
        SandboxError::Unavailable(format!(
            "failed to spawn {bin}: {err}. Install microsandbox and put it on PATH, or set MSB_BIN."
        ))
    })
}

fn msb_failure(action: &str, output: &std::process::Output) -> SandboxError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    SandboxError::Unavailable(format!(
        "msb {action} failed (status {:?}): {detail}",
        output.status.code()
    ))
}

/// `Ok(None)` when the named sandbox is gone. `Ok(Some(true))` when it is running.
pub(crate) fn microsandbox_running(name: &str) -> Result<Option<bool>> {
    let output = run_msb(&["status".into(), name.into()])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let text = format!("{stderr}\n{stdout}").to_ascii_lowercase();
        if text.contains("not found") || text.contains("no such") || text.contains("unknown") {
            return Ok(None);
        }
        return Err(msb_failure("status", &output));
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    Ok(Some(text.contains("running")))
}

pub(crate) async fn microsandbox_doctor() -> Result<()> {
    let output = tokio::task::spawn_blocking(|| run_msb(&["doctor".into()]))
        .await
        .map_err(|err| SandboxError::Unavailable(format!("msb doctor task failed: {err}")))??;
    if output.status.success() {
        Ok(())
    } else {
        Err(msb_failure("doctor", &output))
    }
}

fn reserve_host_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|err| {
        SandboxError::Unavailable(format!("no free host port for microsandbox: {err}"))
    })?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| SandboxError::Unavailable(format!("host port lookup failed: {err}")))
}

pub(crate) async fn create_sidecar_microsandbox(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };
    let token = match token_override {
        Some(token) if !token.trim().is_empty() => token.to_string(),
        _ => crate::auth::generate_token(),
    };
    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let environment = build_container_environment(
        &effective_env,
        &token,
        config.container_port,
        &request.capabilities_json,
    )?;
    let host_port = reserve_host_port()?;
    let memory_mb = request.memory_mb.max(256);
    let cpus = request.cpu_cores.max(1);
    let mut args = vec![
        "create".into(),
        "--name".into(),
        sandbox_id.clone(),
        "--cpus".into(),
        cpus.to_string(),
        "--memory".into(),
        format!("{memory_mb}M"),
        "--port".into(),
        format!("127.0.0.1:{host_port}:{}", config.container_port),
    ];
    for var in &environment.vars {
        args.push("--env".into());
        args.push(var.clone());
    }
    args.push(effective_image.clone());

    let output = tokio::task::spawn_blocking(move || run_msb(&args))
        .await
        .map_err(|err| SandboxError::Unavailable(format!("msb create task failed: {err}")))??;
    if !output.status.success() {
        return Err(msb_failure("create", &output));
    }

    let metadata_json =
        metadata_with_runtime_backend(&request.metadata_json, RuntimeBackend::Microsandbox)?;
    let sidecar_url = format!("http://{}:{host_port}", config.public_host);
    let now = crate::util::now_ts();
    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: sandbox_id.clone(),
        sidecar_url,
        sidecar_port: host_port,
        ssh_port: None,
        token,
        created_at: now,
        cpu_cores: cpus,
        memory_mb,
        state: SandboxState::Running,
        idle_timeout_seconds: config.effective_idle_timeout(request.idle_timeout_seconds),
        max_lifetime_seconds: config.effective_max_lifetime(request.max_lifetime_seconds),
        last_activity_at: now,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: effective_image,
        base_env_json: request.env_json.clone(),
        user_env_json: request.user_env_json.clone(),
        snapshot_destination: None,
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
        extra_ports: HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
        capabilities_json: request.capabilities_json.clone(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed)?;
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(record.cpu_cores, record.memory_mb);
    Ok(record)
}

pub(crate) async fn stop_microsandbox(name: &str) -> Result<()> {
    let name = name.to_string();
    let output = tokio::task::spawn_blocking(move || run_msb(&["stop".into(), name]))
        .await
        .map_err(|err| SandboxError::Unavailable(format!("msb stop task failed: {err}")))??;
    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    if text.contains("not found") || text.contains("already stopped") {
        return Ok(());
    }
    Err(msb_failure("stop", &output))
}

pub(crate) async fn start_microsandbox(name: &str) -> Result<()> {
    let name = name.to_string();
    let output = tokio::task::spawn_blocking(move || run_msb(&["start".into(), name]))
        .await
        .map_err(|err| SandboxError::Unavailable(format!("msb start task failed: {err}")))??;
    if output.status.success() {
        Ok(())
    } else {
        Err(msb_failure("start", &output))
    }
}

pub(crate) async fn delete_microsandbox(name: &str) -> Result<()> {
    let name = name.to_string();
    let output = tokio::task::spawn_blocking(move || run_msb(&["rm".into(), "-f".into(), name]))
        .await
        .map_err(|err| SandboxError::Unavailable(format!("msb rm task failed: {err}")))??;
    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    if text.contains("not found") || text.contains("no such") {
        return Ok(());
    }
    Err(msb_failure("rm", &output))
}
