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

#[cfg(test)]
mod tests {
    use super::*;

    // ── shell_escape ────────────────────────────────────────────────────

    #[test]
    fn shell_escape_empty_string() {
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn shell_escape_normal_string() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_string_with_single_quotes() {
        // Each embedded ' becomes '"'"'
        assert_eq!(shell_escape("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn shell_escape_special_chars() {
        let input = "hello world; rm -rf /";
        let escaped = shell_escape(input);
        assert!(escaped.starts_with('\''));
        assert!(escaped.ends_with('\''));
        // The semicolon and spaces are safely inside quotes
        assert!(escaped.contains("hello world; rm -rf /"));
    }

    #[test]
    fn shell_escape_multiple_single_quotes() {
        let input = "a'b'c";
        let escaped = shell_escape(input);
        assert_eq!(escaped, "'a'\"'\"'b'\"'\"'c'");
    }

    // ── build_snapshot_command ───────────────────────────────────────────

    #[test]
    fn build_snapshot_command_valid_https() {
        let result = build_snapshot_command("https://example.com/snap.tar.gz", true, true);
        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
        assert!(cmd.contains("example.com"));
    }

    #[test]
    fn build_snapshot_command_valid_s3() {
        let result = build_snapshot_command("s3://my-bucket/snap.tar.gz", true, false);
        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(!cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn build_snapshot_command_rejects_private_ip() {
        let result = build_snapshot_command("https://192.168.1.1/snap", true, true);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("private"));
    }

    #[test]
    fn build_snapshot_command_rejects_10_network() {
        let result = build_snapshot_command("https://10.0.0.1/snap", true, true);
        assert!(result.is_err());
    }

    #[test]
    fn build_snapshot_command_rejects_172_private() {
        let result = build_snapshot_command("https://172.16.0.1/snap", true, true);
        assert!(result.is_err());
    }

    #[test]
    fn build_snapshot_command_rejects_localhost() {
        let result = build_snapshot_command("https://localhost/snap", true, true);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("localhost"));
    }

    #[test]
    fn build_snapshot_command_rejects_loopback_ip() {
        let result = build_snapshot_command("https://127.0.0.1/snap", true, true);
        assert!(result.is_err());
    }

    #[test]
    fn build_snapshot_command_rejects_http() {
        let result = build_snapshot_command("http://example.com/snap", true, true);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("https://") || err.contains("s3://"));
    }

    #[test]
    fn build_snapshot_command_rejects_file_scheme() {
        let result = build_snapshot_command("file:///etc/passwd", true, true);
        assert!(result.is_err());
    }

    #[test]
    fn build_snapshot_command_rejects_ftp_scheme() {
        let result = build_snapshot_command("ftp://example.com/snap", true, true);
        assert!(result.is_err());
    }

    #[test]
    fn build_snapshot_command_rejects_empty_paths() {
        let result = build_snapshot_command("https://example.com/snap", false, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("workspace or state"));
    }

    #[test]
    fn build_snapshot_command_workspace_only() {
        let result = build_snapshot_command("https://example.com/snap", true, false);
        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert!(cmd.contains("/home/agent"));
        assert!(!cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn build_snapshot_command_state_only() {
        let result = build_snapshot_command("https://example.com/snap", false, true);
        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert!(!cmd.contains("/home/agent"));
        assert!(cmd.contains("/var/lib/sidecar"));
    }

    #[test]
    fn build_snapshot_command_rejects_link_local() {
        let result = build_snapshot_command("https://169.254.169.254/snap", true, true);
        assert!(result.is_err());
    }

    // ── normalize_username ──────────────────────────────────────────────

    #[test]
    fn normalize_username_empty_defaults_to_root() {
        assert_eq!(normalize_username("").unwrap(), "root");
    }

    #[test]
    fn normalize_username_whitespace_defaults_to_root() {
        assert_eq!(normalize_username("   ").unwrap(), "root");
    }

    #[test]
    fn normalize_username_valid() {
        assert_eq!(normalize_username("alice").unwrap(), "alice");
    }

    #[test]
    fn normalize_username_with_dash_underscore_dot() {
        assert_eq!(
            normalize_username("my-user_name.1").unwrap(),
            "my-user_name.1"
        );
    }

    #[test]
    fn normalize_username_rejects_at_symbol() {
        assert!(normalize_username("user@host").is_err());
    }

    #[test]
    fn normalize_username_rejects_spaces() {
        assert!(normalize_username("user name").is_err());
    }

    #[test]
    fn normalize_username_rejects_semicolon() {
        assert!(normalize_username("user;evil").is_err());
    }

    #[test]
    fn normalize_username_rejects_slash() {
        assert!(normalize_username("../root").is_err());
    }

    // ── parse_json_object ───────────────────────────────────────────────

    #[test]
    fn parse_json_object_empty_string() {
        let result = parse_json_object("", "test").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_whitespace_only() {
        let result = parse_json_object("   ", "test").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_object_valid_object() {
        let result = parse_json_object(r#"{"key": "value"}"#, "test").unwrap();
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn parse_json_object_rejects_array() {
        let result = parse_json_object("[1, 2, 3]", "test");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must be a JSON object"));
    }

    #[test]
    fn parse_json_object_rejects_string() {
        let result = parse_json_object(r#""hello""#, "test");
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_object_invalid_json() {
        let result = parse_json_object("{bad json}", "test");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not valid JSON"));
    }

    // ── merge_metadata ──────────────────────────────────────────────────

    #[test]
    fn merge_metadata_no_image_no_stack() {
        let metadata = Some(serde_json::json!({"existing": true}));
        let result = merge_metadata(metadata.clone(), "", "").unwrap();
        // Returns original metadata unchanged
        assert_eq!(result, metadata);
    }

    #[test]
    fn merge_metadata_none_with_no_image_no_stack() {
        let result = merge_metadata(None, "", "").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn merge_metadata_with_image() {
        let result = merge_metadata(None, "ubuntu:22.04", "").unwrap();
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["image"], "ubuntu:22.04");
        assert!(val.get("stack").is_none());
    }

    #[test]
    fn merge_metadata_with_stack() {
        let result = merge_metadata(None, "", "python").unwrap();
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["stack"], "python");
        assert!(val.get("image").is_none());
    }

    #[test]
    fn merge_metadata_with_both() {
        let existing = Some(serde_json::json!({"version": 1}));
        let result = merge_metadata(existing, "ubuntu:22.04", "python").unwrap();
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["image"], "ubuntu:22.04");
        assert_eq!(val["stack"], "python");
        assert_eq!(val["version"], 1);
    }

    #[test]
    fn merge_metadata_non_object_errors() {
        let metadata = Some(serde_json::json!([1, 2, 3]));
        let result = merge_metadata(metadata, "ubuntu:22.04", "");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must be a JSON object"));
    }

    #[test]
    fn merge_metadata_string_value_errors() {
        let metadata = Some(serde_json::json!("just a string"));
        let result = merge_metadata(metadata, "img", "");
        assert!(result.is_err());
    }
}
