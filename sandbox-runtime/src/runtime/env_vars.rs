use super::*;

/// Parse the `capabilities_json` field into the comma-separated wire
/// format the sidecar's `SIDECAR_CAPABILITIES` parser expects.
///
/// JSON array on input, comma-separated list on the env var, and unknown
/// entries dropped silently. Returns `None` when nothing recognizable is
/// present so callers can skip the env-var injection entirely.
pub(crate) fn parse_sidecar_capabilities(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Accept either a JSON array (the on-wire form) or a plain
    // comma-separated list as a convenience for direct callers.
    let entries: Vec<String> = if trimmed.starts_with('[') {
        match serde_json::from_str::<Vec<String>>(trimmed) {
            Ok(v) => v,
            Err(_) => return None,
        }
    } else {
        trimmed
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let known: Vec<String> = entries
        .into_iter()
        .filter(|c| c == "computer_use" || c == "all_harness")
        .collect();
    if known.is_empty() {
        None
    } else {
        Some(known.join(","))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared Docker helpers — used by create, snapshot-resume, and S3-restore paths
// ─────────────────────────────────────────────────────────────────────────────

/// Merge base and user env JSON strings into a single JSON object string.
/// User values override base values when keys collide.
/// Default Tangle Intelligence base the sidecar telemetry sink posts to.
pub(crate) const DEFAULT_INTELLIGENCE_ENDPOINT: &str = "https://intelligence.tangle.tools";

/// Fill a sidecar's env with the operator's Tangle Intelligence telemetry
/// config so its agent-runtime loop OTEL exports to Intelligence. No-op unless
/// the operator process carries `TANGLE_API_KEY`. Never overrides explicit
/// per-sandbox telemetry env (`TELEMETRY_*` / `OTEL_EXPORTER_OTLP_ENDPOINT`).
pub(crate) fn inject_intelligence_telemetry_env(env: &mut HashMap<String, String>) {
    // Respect an explicit telemetry/OTLP setup on the sandbox.
    if env.contains_key("TELEMETRY_API_KEY") || env.contains_key("OTEL_EXPORTER_OTLP_ENDPOINT") {
        return;
    }
    let Ok(key) = env::var("TANGLE_API_KEY") else {
        return;
    };
    if key.trim().is_empty() {
        return;
    }
    env.entry("TELEMETRY_ENABLED".to_string())
        .or_insert_with(|| "true".to_string());
    env.entry("TELEMETRY_ENDPOINT".to_string())
        .or_insert_with(|| {
            env::var("TELEMETRY_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_INTELLIGENCE_ENDPOINT.to_string())
        });
    env.entry("TELEMETRY_API_KEY".to_string()).or_insert(key);
}

pub fn merge_env_json(base: &str, user: &str) -> String {
    let user_trimmed = user.trim();
    if user_trimmed.is_empty() || user_trimmed == "{}" {
        return base.to_string();
    }
    let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(base)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "Failed to parse base_env_json, using empty map");
            serde_json::Map::new()
        });
    if let Ok(user_map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(user) {
        map.extend(user_map);
    }
    serde_json::to_string(&map).unwrap_or_else(|e| {
        tracing::error!(error = %e, "Failed to serialize merged env JSON, returning empty");
        "{}".to_string()
    })
}

pub fn workflow_runtime_credentials_available(env_json: &str) -> Result<bool> {
    let env_map = parse_json_object(env_json, "env_json")?;
    let Some(Value::Object(map)) = env_map else {
        return Ok(false);
    };

    let has_native_provider_key = map
        .get("ANTHROPIC_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || map
            .get("ZAI_API_KEY")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());

    let has_explicit_opencode = map
        .get("OPENCODE_MODEL_PROVIDER")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        && map
            .get("OPENCODE_MODEL_NAME")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && map
            .get("OPENCODE_MODEL_API_KEY")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());

    Ok(has_native_provider_key || has_explicit_opencode)
}

/// Build the `Vec<String>` of `KEY=VALUE` env vars for a Docker container.
pub(crate) fn build_env_vars(
    env_json: &str,
    token: &str,
    container_port: u16,
    capabilities_json: &str,
) -> Result<Vec<String>> {
    let mut env_vars = vec![
        format!("SIDECAR_PORT={container_port}"),
        format!("SIDECAR_AUTH_TOKEN={token}"),
        // Switch sidecar to container mode so it uses /home/agent (where the
        // Dockerfile pre-creates .local, .cache, .config owned by agent) instead
        // of per-request /tmp/agent/workspace/req-* dirs on tmpfs.
        "AGENT_WORKSPACE_ROOT=/home/agent".to_string(),
        "AGENT_SUBPROCESS_UID=1000".to_string(),
        "AGENT_SUBPROCESS_GID=1000".to_string(),
    ];

    // Sidecar capabilities (e.g. `computer_use`). Inject before user env
    // so a malformed user-supplied SIDECAR_CAPABILITIES override would
    // win — but in practice users do not set this and the env-var name
    // is documented as runtime-controlled.
    if let Some(caps) = parse_sidecar_capabilities(capabilities_json) {
        env_vars.push(format!("SIDECAR_CAPABILITIES={caps}"));
    }

    // User-supplied env vars are appended after defaults so they can override.
    let env_map = parse_json_object(env_json, "env_json")?;
    if let Some(Value::Object(map)) = env_map.as_ref() {
        for (key, value) in map {
            let val = match value {
                Value::String(v) => v.clone(),
                Value::Number(v) => v.to_string(),
                Value::Bool(v) => v.to_string(),
                _ => continue,
            };
            env_vars.push(format!("{key}={val}"));
        }
    }
    Ok(env_vars)
}
