use once_cell::sync::OnceCell;
use reqwest::Client;
use serde_json::{Map, Value};
use std::time::Duration;

static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();

pub fn http_client(timeout: Duration) -> Result<&'static Client, String> {
    HTTP_CLIENT
        .get_or_try_init(|| {
            Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|err| format!("Failed to build HTTP client: {err}"))
        })
        .map_err(|err| err.to_string())
}

pub fn parse_json_object(value: &str, field_name: &str) -> Result<Option<Value>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|err| format!("{field_name} is not valid JSON: {err}"))?;

    if !parsed.is_object() {
        return Err(format!("{field_name} must be a JSON object"));
    }

    Ok(Some(parsed))
}

pub fn merge_metadata(
    mut metadata: Option<Value>,
    image: &str,
    stack: &str,
) -> Result<Option<Value>, String> {
    if image.is_empty() && stack.is_empty() {
        return Ok(metadata);
    }

    let mut object = match metadata.take() {
        Some(Value::Object(map)) => map,
        Some(_) => return Err("metadata_json must be a JSON object".to_string()),
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

pub fn normalize_username(username: &str) -> Result<String, String> {
    let trimmed = username.trim();
    let name = if trimmed.is_empty() { "root" } else { trimmed };
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err("Invalid SSH username".to_string());
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
) -> Result<String, String> {
    let mut paths = Vec::new();
    if include_workspace {
        paths.push("/workspace");
    }
    if include_state {
        paths.push("/var/lib/sidecar");
    }
    if paths.is_empty() {
        return Err("Snapshot must include workspace or state".to_string());
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
