use super::*;

pub(crate) fn parse_runtime_backend_value(value: &str) -> Option<RuntimeBackend> {
    match value.trim().to_ascii_lowercase().as_str() {
        "docker" | "container" => Some(RuntimeBackend::Docker),
        "firecracker" | "microvm" => Some(RuntimeBackend::Firecracker),
        "tee" | "confidential" | "confidential-vm" => Some(RuntimeBackend::Tee),
        _ => None,
    }
}

pub(crate) fn runtime_backend_name(backend: RuntimeBackend) -> &'static str {
    match backend {
        RuntimeBackend::Docker => "docker",
        RuntimeBackend::Firecracker => "firecracker",
        RuntimeBackend::Tee => "tee",
    }
}

pub(crate) fn metadata_with_runtime_backend(
    metadata_json: &str,
    backend: RuntimeBackend,
) -> Result<String> {
    let mut map = match parse_json_object(metadata_json, "metadata_json")? {
        Some(Value::Object(map)) => map,
        None => Map::new(),
        Some(_) => {
            return Err(SandboxError::Validation(
                "metadata_json must be a JSON object".into(),
            ));
        }
    };

    map.insert(
        "runtime_backend".to_string(),
        Value::String(runtime_backend_name(backend).to_string()),
    );

    serde_json::to_string(&Value::Object(map))
        .map_err(|e| SandboxError::Validation(format!("failed to serialize metadata_json: {e}")))
}

pub(crate) fn parse_runtime_backend_from_metadata(
    metadata_json: &str,
) -> Result<Option<RuntimeBackend>> {
    let metadata = parse_json_object(metadata_json, "metadata_json")?;
    let Some(meta) = metadata else {
        return Ok(None);
    };

    let backend = meta
        .get("runtime_backend")
        .and_then(|v| v.as_str())
        .or_else(|| {
            meta.get("runtime")
                .and_then(|v| v.get("backend"))
                .and_then(|v| v.as_str())
        });

    let Some(raw) = backend else {
        return Ok(None);
    };

    parse_runtime_backend_value(raw).map(Some).ok_or_else(|| {
        SandboxError::Validation(format!(
            "metadata_json.runtime_backend must be one of: docker, firecracker, tee (got '{raw}')"
        ))
    })
}

pub(crate) fn parse_runtime_backend_from_env() -> Result<RuntimeBackend> {
    let raw = std::env::var("SANDBOX_RUNTIME_BACKEND").unwrap_or_else(|_| "docker".to_string());
    parse_runtime_backend_value(&raw).ok_or_else(|| {
        SandboxError::Validation(format!(
            "SANDBOX_RUNTIME_BACKEND must be one of: docker, firecracker, tee (got '{raw}')"
        ))
    })
}

pub(crate) fn resolve_runtime_backend(request: &CreateSandboxParams) -> Result<RuntimeBackend> {
    let metadata_backend = parse_runtime_backend_from_metadata(&request.metadata_json)?;
    let selected = match metadata_backend {
        Some(b) => b,
        None => parse_runtime_backend_from_env()?,
    };

    let tee_required = request.tee_config.as_ref().is_some_and(|cfg| cfg.required);
    if tee_required {
        if selected == RuntimeBackend::Firecracker {
            return Err(SandboxError::Validation(
                "runtime_backend=firecracker is incompatible with tee_required=true".into(),
            ));
        }
        return Ok(RuntimeBackend::Tee);
    }

    Ok(selected)
}

pub(crate) fn runtime_backend_for_record(record: &SandboxRecord) -> RuntimeBackend {
    if record.tee_deployment_id.is_some() {
        return RuntimeBackend::Tee;
    }
    match parse_runtime_backend_from_metadata(&record.metadata_json) {
        Ok(Some(backend)) => backend,
        Ok(None) => RuntimeBackend::Docker,
        Err(err) => {
            tracing::warn!(
                sandbox_id = %record.id,
                error = %err,
                "invalid metadata_json.runtime_backend on stored record; defaulting to docker backend"
            );
            RuntimeBackend::Docker
        }
    }
}

pub(crate) fn record_uses_firecracker(record: &SandboxRecord) -> bool {
    runtime_backend_for_record(record) == RuntimeBackend::Firecracker
}

pub fn supports_docker_endpoint_refresh(record: &SandboxRecord) -> bool {
    record.tee_deployment_id.is_none() && !record_uses_firecracker(record)
}

pub(crate) fn parse_url_port(url: &str) -> Option<u16> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.port_or_known_default())
}
