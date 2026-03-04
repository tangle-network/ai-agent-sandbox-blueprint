//! Serde-based request/response types for the operator HTTP API.
//!
//! These parallel the `sol!` ABI types in `instance_types.rs` but use
//! serde for JSON serialization — needed because `sol!` structs don't
//! implement `Serialize`/`Deserialize`.

use serde::{Deserialize, Serialize};

/// Maximum allowed length for command/prompt/message strings (100 KB).
const MAX_TEXT_LEN: usize = 100 * 1024;
#[cfg(test)]
const MAX_SSH_KEY_LEN: usize = crate::ssh_validation::MAX_SSH_KEY_LEN;
#[cfg(test)]
const MAX_USERNAME_LEN: usize = crate::ssh_validation::MAX_USERNAME_LEN;

/// Maximum number of secret keys.
const MAX_SECRET_KEYS: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Validation helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Validate that a string is not empty and within max length.
fn validate_required(field: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_len {
        return Err(format!("{field} exceeds maximum length ({max_len} bytes)"));
    }
    Ok(())
}

/// Validate username (alphanumeric, dashes, underscores, dots; max 32 chars).
fn validate_username(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Ok(());
    }
    crate::ssh_validation::validate_ssh_username(name)
}

/// Validate SSH public key format.
fn validate_ssh_public_key(key: &str) -> Result<(), String> {
    crate::ssh_validation::validate_ssh_public_key(key)
}

// ─────────────────────────────────────────────────────────────────────────────
// Exec
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecApiRequest {
    pub command: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub env_json: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl ExecApiRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_required("command", &self.command, MAX_TEXT_LEN)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PromptApiRequest {
    pub message: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub context_json: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl PromptApiRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_required("message", &self.message, MAX_TEXT_LEN)
    }
}

#[derive(Debug, Serialize)]
pub struct ExecApiResponse {
    pub exit_code: u32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize)]
pub struct PromptApiResponse {
    pub success: bool,
    pub response: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub trace_id: String,
    pub duration_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Task
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TaskApiRequest {
    pub prompt: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub max_turns: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub context_json: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl TaskApiRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_required("prompt", &self.prompt, MAX_TEXT_LEN)
    }
}

#[derive(Debug, Serialize)]
pub struct TaskApiResponse {
    pub success: bool,
    pub result: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub trace_id: String,
    pub duration_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub session_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SnapshotApiRequest {
    pub destination: String,
    #[serde(default)]
    pub include_workspace: bool,
    #[serde(default)]
    pub include_state: bool,
}

#[derive(Debug, Serialize)]
pub struct SnapshotApiResponse {
    pub success: bool,
    pub result: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// SSH
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SshProvisionApiRequest {
    #[serde(default = "default_ssh_username")]
    pub username: String,
    pub public_key: String,
}

fn default_ssh_username() -> String {
    "agent".to_string()
}

impl SshProvisionApiRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_username(&self.username)?;
        validate_ssh_public_key(&self.public_key)
    }
}

#[derive(Debug, Deserialize)]
pub struct SshRevokeApiRequest {
    #[serde(default = "default_ssh_username")]
    pub username: String,
    pub public_key: String,
}

impl SshRevokeApiRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_username(&self.username)?;
        validate_ssh_public_key(&self.public_key)
    }
}

#[derive(Debug, Serialize)]
pub struct SshApiResponse {
    pub success: bool,
    pub result: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// Secrets validation
// ─────────────────────────────────────────────────────────────────────────────

/// Validate a secrets map (max keys, no excessively large values).
pub fn validate_secrets_map(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if map.is_empty() {
        return Err("env_json must contain at least one key".into());
    }
    if map.len() > MAX_SECRET_KEYS {
        return Err(format!(
            "env_json exceeds maximum of {MAX_SECRET_KEYS} keys"
        ));
    }
    for (key, val) in map {
        if key.is_empty() {
            return Err("secret keys must not be empty".into());
        }
        if key.len() > 256 {
            return Err(format!("secret key '{key}' exceeds max length (256 chars)"));
        }
        // Estimate value size
        let val_str = val.to_string();
        if val_str.len() > 64 * 1024 {
            return Err(format!("secret value for '{key}' exceeds max size (64 KB)"));
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Stop / Resume (no request body needed)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct LifecycleApiResponse {
    pub success: bool,
    pub sandbox_id: String,
    pub state: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── validate_required ───────────────────────────────────────────────

    #[test]
    fn validate_required_empty() {
        assert!(validate_required("f", "", 100).is_err());
    }

    #[test]
    fn validate_required_whitespace_only() {
        assert!(validate_required("f", "   \t\n", 100).is_err());
    }

    #[test]
    fn validate_required_at_limit() {
        let s = "a".repeat(100);
        assert!(validate_required("f", &s, 100).is_ok());
    }

    #[test]
    fn validate_required_over_limit() {
        let s = "a".repeat(101);
        assert!(validate_required("f", &s, 100).is_err());
    }

    #[test]
    fn validate_required_valid() {
        assert!(validate_required("f", "hello", 100).is_ok());
    }

    // ── validate_ssh_public_key ─────────────────────────────────────────

    #[test]
    fn ssh_key_empty() {
        assert!(validate_ssh_public_key("").is_err());
    }

    #[test]
    fn ssh_key_too_long() {
        let key = format!("ssh-ed25519 {}", "A".repeat(MAX_SSH_KEY_LEN));
        assert!(validate_ssh_public_key(&key).is_err());
    }

    #[test]
    fn ssh_key_invalid_prefix() {
        assert!(validate_ssh_public_key("pgp-key AAAA").is_err());
    }

    #[test]
    fn ssh_key_missing_data() {
        assert!(validate_ssh_public_key("ssh-ed25519").is_err());
    }

    #[test]
    fn ssh_key_valid_ed25519() {
        assert!(validate_ssh_public_key("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest").is_ok());
    }

    #[test]
    fn ssh_key_valid_rsa() {
        assert!(validate_ssh_public_key("ssh-rsa AAAAB3NzaC1yc2EAAAATest user@host").is_ok());
    }

    // ── validate_username ───────────────────────────────────────────────

    #[test]
    fn username_empty_defaults_ok() {
        assert!(validate_username("").is_ok());
    }

    #[test]
    fn username_too_long() {
        let name = "a".repeat(MAX_USERNAME_LEN + 1);
        assert!(validate_username(&name).is_err());
    }

    #[test]
    fn username_invalid_at_sign() {
        assert!(validate_username("user@host").is_err());
    }

    #[test]
    fn username_invalid_spaces() {
        assert!(validate_username("my user").is_err());
    }

    #[test]
    fn username_valid_with_special() {
        assert!(validate_username("my-user_1.0").is_ok());
    }

    #[test]
    fn username_at_limit() {
        let name = "a".repeat(MAX_USERNAME_LEN);
        assert!(validate_username(&name).is_ok());
    }

    // ── validate_secrets_map ────────────────────────────────────────────

    #[test]
    fn secrets_empty_map() {
        let map = serde_json::Map::new();
        assert!(validate_secrets_map(&map).is_err());
    }

    #[test]
    fn secrets_too_many_keys() {
        let mut map = serde_json::Map::new();
        for i in 0..=MAX_SECRET_KEYS {
            map.insert(format!("key{i}"), json!("val"));
        }
        assert!(validate_secrets_map(&map).is_err());
    }

    #[test]
    fn secrets_empty_key() {
        let mut map = serde_json::Map::new();
        map.insert(String::new(), json!("val"));
        assert!(validate_secrets_map(&map).is_err());
    }

    #[test]
    fn secrets_key_too_long() {
        let mut map = serde_json::Map::new();
        map.insert("k".repeat(257), json!("val"));
        assert!(validate_secrets_map(&map).is_err());
    }

    #[test]
    fn secrets_value_too_large() {
        let mut map = serde_json::Map::new();
        map.insert("key".into(), json!("x".repeat(64 * 1024 + 1)));
        assert!(validate_secrets_map(&map).is_err());
    }

    #[test]
    fn secrets_valid_map() {
        let mut map = serde_json::Map::new();
        map.insert("API_KEY".into(), json!("sk-test123"));
        map.insert("DB_URL".into(), json!("postgres://localhost/db"));
        assert!(validate_secrets_map(&map).is_ok());
    }

    // ── Request validate() ──────────────────────────────────────────────

    #[test]
    fn exec_request_empty_command() {
        let req = ExecApiRequest {
            command: String::new(),
            session_id: String::new(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn exec_request_valid() {
        let req = ExecApiRequest {
            command: "ls -la".into(),
            session_id: String::new(),
            cwd: String::new(),
            env_json: String::new(),
            timeout_ms: 0,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn ssh_provision_invalid_key() {
        let req = SshProvisionApiRequest {
            username: "agent".into(),
            public_key: "not-a-key".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn ssh_provision_invalid_username() {
        let req = SshProvisionApiRequest {
            username: "bad user!".into(),
            public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn ssh_provision_valid() {
        let req = SshProvisionApiRequest {
            username: "agent".into(),
            public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest".into(),
        };
        assert!(req.validate().is_ok());
    }
}
