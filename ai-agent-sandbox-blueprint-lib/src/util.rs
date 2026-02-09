use once_cell::sync::OnceCell;
use reqwest::Client;
use serde_json::{Map, Value};

use crate::error::{Result, SandboxError};

static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();

/// Get the shared HTTP client. The timeout is set from `SidecarRuntimeConfig`
/// on first initialization and reused for all subsequent calls.
pub fn http_client() -> Result<&'static Client> {
    HTTP_CLIENT
        .get_or_try_init(|| {
            let config = crate::runtime::SidecarRuntimeConfig::load();
            Client::builder()
                .timeout(config.timeout)
                .build()
                .map_err(|err| SandboxError::Http(format!("Failed to build HTTP client: {err}")))
        })
        .map_err(|err| SandboxError::Http(err.to_string()))
}

pub fn parse_json_object(value: &str, field_name: &str) -> Result<Option<Value>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed: Value = serde_json::from_str(trimmed).map_err(|err| {
        SandboxError::Validation(format!("{field_name} is not valid JSON: {err}"))
    })?;

    if !parsed.is_object() {
        return Err(SandboxError::Validation(format!(
            "{field_name} must be a JSON object"
        )));
    }

    Ok(Some(parsed))
}

pub fn merge_metadata(
    mut metadata: Option<Value>,
    image: &str,
    stack: &str,
) -> Result<Option<Value>> {
    if image.is_empty() && stack.is_empty() {
        return Ok(metadata);
    }

    let mut object = match metadata.take() {
        Some(Value::Object(map)) => map,
        Some(_) => {
            return Err(SandboxError::Validation(
                "metadata_json must be a JSON object".into(),
            ));
        }
        None => Map::new(),
    };

    if !image.is_empty() {
        object.insert("image".to_string(), Value::String(image.to_string()));
    }

    if !stack.is_empty() {
        object.insert("stack".to_string(), Value::String(stack.to_string()));
    }

    Ok(Some(Value::Object(object)))
}

pub fn normalize_username(username: &str) -> Result<String> {
    let trimmed = username.trim();
    let name = if trimmed.is_empty() { "root" } else { trimmed };
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(SandboxError::Validation("Invalid SSH username".into()));
    }
    Ok(name.to_string())
}

pub fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

pub fn build_snapshot_command(
    destination: &str,
    include_workspace: bool,
    include_state: bool,
) -> Result<String> {
    let mut paths = Vec::new();
    if include_workspace {
        paths.push("/workspace");
    }
    if include_state {
        paths.push("/var/lib/sidecar");
    }
    if paths.is_empty() {
        return Err(SandboxError::Validation(
            "Snapshot must include workspace or state".into(),
        ));
    }

    let dest = shell_escape(destination);
    let targets = paths.join(" ");
    Ok(format!(
        "set -euo pipefail; tmp=$(mktemp /tmp/sandbox-snapshot.XXXXXX.tar.gz); \
 tar -czf \"$tmp\" {targets}; \
 curl -fsSL -X PUT --upload-file \"$tmp\" {dest}; \
 rm -f \"$tmp\""
    ))
}
