use super::*;

const DEFAULT_WORKSPACE_ROOT: &str = "/home/agent";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContainerEnvironment {
    pub vars: Vec<String>,
    pub workspace_root: String,
}

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

fn normalize_workspace_root(
    env_vars: &mut std::collections::BTreeMap<String, String>,
) -> Result<String> {
    let requested = env_vars
        .get("AGENT_WORKSPACE_ROOT")
        .map(String::as_str)
        .unwrap_or(DEFAULT_WORKSPACE_ROOT)
        .trim();
    let path = std::path::Path::new(requested);
    let has_only_normal_components = path.components().enumerate().all(|(index, component)| {
        matches!(
            (index, component),
            (0, std::path::Component::RootDir) | (_, std::path::Component::Normal(_))
        )
    });
    if requested.is_empty()
        || !path.is_absolute()
        || !has_only_normal_components
        || !path.starts_with(DEFAULT_WORKSPACE_ROOT)
    {
        return Err(SandboxError::Validation(format!(
            "AGENT_WORKSPACE_ROOT must be /home/agent or a directory below it, got {requested:?}"
        )));
    }

    if let Some(workspace_path) = env_vars.get("WORKSPACE_PATH") {
        let workspace_path = workspace_path.trim();
        if workspace_path != DEFAULT_WORKSPACE_ROOT {
            return Err(SandboxError::Validation(format!(
                "WORKSPACE_PATH must be /home/agent when supplied, got {workspace_path:?}"
            )));
        }
    }

    let normalized = path.to_string_lossy().into_owned();
    env_vars.insert("AGENT_WORKSPACE_ROOT".to_string(), normalized.clone());
    Ok(normalized)
}

fn is_runtime_owned_env_key(key: &str) -> bool {
    key == "SIDECAR_PORT"
        || key == "SIDECAR_CAPABILITIES"
        || key == "AGENT_SUBPROCESS_UID"
        || key == "AGENT_SUBPROCESS_GID"
        || key == "AGENT_SUBPROCESS_USER"
        || key.starts_with("SIDECAR_AUTH_")
}

/// Build one authoritative environment for a Docker container.
pub(crate) fn build_container_environment(
    env_json: &str,
    token: &str,
    container_port: u16,
    capabilities_json: &str,
) -> Result<ContainerEnvironment> {
    let env_map = parse_json_object(env_json, "env_json")?;
    let mut env_vars = std::collections::BTreeMap::new();
    if let Some(Value::Object(map)) = env_map.as_ref() {
        for (key, value) in map {
            let val = match value {
                Value::String(v) => v.clone(),
                Value::Number(v) => v.to_string(),
                Value::Bool(v) => v.to_string(),
                _ => continue,
            };
            env_vars.insert(key.clone(), val);
        }
    }

    let workspace_root = normalize_workspace_root(&mut env_vars)?;

    // Caller env cannot weaken the operator-to-sidecar boundary. Remove every
    // alternate auth control before adding the one token the operator owns.
    env_vars.retain(|key, _| !is_runtime_owned_env_key(key));

    // These values define the operator-to-sidecar security boundary. They are
    // authoritative even if an untrusted env_json contains the same keys.
    env_vars.insert("SIDECAR_PORT".to_string(), container_port.to_string());
    env_vars.insert("SIDECAR_AUTH_TOKEN".to_string(), token.to_string());
    env_vars.insert("AGENT_SUBPROCESS_UID".to_string(), "1000".to_string());
    env_vars.insert("AGENT_SUBPROCESS_GID".to_string(), "1000".to_string());
    if let Some(caps) = parse_sidecar_capabilities(capabilities_json) {
        env_vars.insert("SIDECAR_CAPABILITIES".to_string(), caps);
    }

    Ok(ContainerEnvironment {
        vars: env_vars
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
        workspace_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(environment: ContainerEnvironment) -> Vec<String> {
        environment.vars
    }

    #[test]
    fn container_env_includes_scalar_values() {
        let vars = values(
            build_container_environment(r#"{"API_KEY":"sk-test","DEBUG":true}"#, "tok", 8080, "")
                .unwrap(),
        );
        assert!(vars.contains(&"API_KEY=sk-test".to_string()));
        assert!(vars.contains(&"DEBUG=true".to_string()));
        assert!(vars.contains(&"SIDECAR_PORT=8080".to_string()));
    }

    #[test]
    fn container_env_rejects_invalid_json() {
        assert!(build_container_environment("not-json", "tok", 3000, "").is_err());
    }

    #[test]
    fn container_env_preserves_explicit_ai_env() {
        let vars = values(
            build_container_environment(r#"{"ZAI_API_KEY":"user-key"}"#, "tok", 8080, "").unwrap(),
        );
        assert!(vars.contains(&"ZAI_API_KEY=user-key".to_string()));
        assert!(!vars.contains(&"OPENCODE_MODEL_API_KEY=user-key".to_string()));
    }

    #[test]
    fn container_env_keeps_one_requested_workspace_root() {
        let environment = build_container_environment(
            r#"{"AGENT_WORKSPACE_ROOT":"/home/agent/vault","WORKSPACE_PATH":"/home/agent"}"#,
            "tok",
            8080,
            "",
        )
        .unwrap();
        assert_eq!(environment.workspace_root, "/home/agent/vault");
        assert_eq!(
            environment
                .vars
                .iter()
                .filter(|value| value.starts_with("AGENT_WORKSPACE_ROOT="))
                .count(),
            1
        );
        assert!(
            environment
                .vars
                .contains(&"AGENT_WORKSPACE_ROOT=/home/agent/vault".to_string())
        );
    }

    #[test]
    fn container_env_rejects_workspace_escape() {
        for env_json in [
            r#"{"AGENT_WORKSPACE_ROOT":"/"}"#,
            r#"{"AGENT_WORKSPACE_ROOT":"/home/agent/../root"}"#,
            r#"{"AGENT_WORKSPACE_ROOT":"relative"}"#,
            r#"{"AGENT_WORKSPACE_ROOT":"/home/agent/vault","WORKSPACE_PATH":"/"}"#,
        ] {
            assert!(
                build_container_environment(env_json, "tok", 8080, "").is_err(),
                "accepted {env_json}"
            );
        }
    }

    #[test]
    fn container_env_keeps_runtime_values_authoritative() {
        let vars = values(
            build_container_environment(
                r#"{"SIDECAR_PORT":"1","SIDECAR_AUTH_TOKEN":"attacker","SIDECAR_AUTH_DISABLED":true,"SIDECAR_AUTH_TOKENS":"attacker","SIDECAR_CAPABILITIES":"computer_use","AGENT_SUBPROCESS_UID":"0","AGENT_SUBPROCESS_GID":"0"}"#,
                "operator-token",
                8080,
                "",
            )
            .unwrap(),
        );

        for expected in [
            "SIDECAR_PORT=8080",
            "SIDECAR_AUTH_TOKEN=operator-token",
            "AGENT_SUBPROCESS_UID=1000",
            "AGENT_SUBPROCESS_GID=1000",
        ] {
            let key = expected.split_once('=').unwrap().0;
            let matches: Vec<_> = vars
                .iter()
                .filter(|value| value.starts_with(&format!("{key}=")))
                .collect();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].as_str(), expected);
        }
        assert!(
            !vars
                .iter()
                .any(|value| value == "SIDECAR_AUTH_DISABLED=true")
        );
        assert!(
            !vars
                .iter()
                .any(|value| value == "SIDECAR_AUTH_TOKENS=attacker")
        );
        assert!(
            !vars
                .iter()
                .any(|value| value.starts_with("SIDECAR_CAPABILITIES="))
        );
    }

    #[test]
    fn container_env_injects_valid_capabilities() {
        let vars =
            values(build_container_environment("{}", "tok", 8080, r#"["computer_use"]"#).unwrap());
        assert!(vars.contains(&"SIDECAR_CAPABILITIES=computer_use".to_string()));
    }
}
