use chrono::Utc;
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

/// Current UTC timestamp as seconds since epoch.
pub fn now_ts() -> u64 {
    Utc::now().timestamp().max(0) as u64
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

/// Validate a snapshot destination URL against SSRF risks.
///
/// Rejects:
/// - Non-HTTPS/S3 schemes (file://, ftp://, gopher://, etc.)
/// - Private/loopback IP addresses (169.254.x.x, 10.x.x.x, 172.16-31.x.x, 192.168.x.x, 127.x.x.x)
/// - `localhost` hostname
fn validate_snapshot_destination(destination: &str) -> Result<()> {
    let trimmed = destination.trim();

    // Allow s3:// URIs (handled by the sidecar's S3 client, not curl)
    if trimmed.starts_with("s3://") {
        return Ok(());
    }

    // Require https:// scheme
    if !trimmed.starts_with("https://") {
        return Err(SandboxError::Validation(
            "Snapshot destination must use https:// or s3:// scheme".into(),
        ));
    }

    // Extract the host portion (between :// and the next / or end)
    let after_scheme = &trimmed["https://".len()..];
    let host = after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    // Block localhost
    if host.eq_ignore_ascii_case("localhost") {
        return Err(SandboxError::Validation(
            "Snapshot destination must not target localhost".into(),
        ));
    }

    // Block private/link-local IP addresses
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local()
                    // Cloud metadata: 169.254.169.254
                    || v4.octets()[0] == 169
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
        if is_private {
            return Err(SandboxError::Validation(
                "Snapshot destination must not target private/internal IP addresses".into(),
            ));
        }
    }

    Ok(())
}

pub fn build_snapshot_command(
    destination: &str,
    include_workspace: bool,
    include_state: bool,
) -> Result<String> {
    validate_snapshot_destination(destination)?;

    let mut paths = Vec::new();
    if include_workspace {
        paths.push("/home/agent");
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
        "set -euo pipefail; tmp=$(mktemp /tmp/snapshot-XXXXXX); \
 tar -czf \"$tmp\" {targets}; \
 curl -fsSL -X PUT --upload-file \"$tmp\" {dest}; \
 rm -f \"$tmp\""
    ))
}
