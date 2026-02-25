//! Serde-based request/response types for the operator HTTP API.
//!
//! These parallel the `sol!` ABI types in `instance_types.rs` but use
//! serde for JSON serialization — needed because `sol!` structs don't
//! implement `Serialize`/`Deserialize`.

use serde::{Deserialize, Serialize};

/// Maximum allowed length for command/prompt/message strings (100 KB).
const MAX_TEXT_LEN: usize = 100 * 1024;

/// Maximum allowed SSH public key length (16 KB).
const MAX_SSH_KEY_LEN: usize = 16 * 1024;

/// Maximum username length.
const MAX_USERNAME_LEN: usize = 32;

/// Maximum number of secret keys.
const MAX_SECRET_KEYS: usize = 256;

/// Accepted SSH key type prefixes.
const SSH_KEY_PREFIXES: &[&str] = &[
    "ssh-rsa ",
    "ssh-ed25519 ",
    "ssh-dss ",
    "ecdsa-sha2-nistp256 ",
    "ecdsa-sha2-nistp384 ",
    "ecdsa-sha2-nistp521 ",
    "sk-ssh-ed25519@openssh.com ",
    "sk-ecdsa-sha2-nistp256@openssh.com ",
];

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

/// Validate SSH public key format.
fn validate_ssh_public_key(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("public_key is required".into());
    }
    if trimmed.len() > MAX_SSH_KEY_LEN {
        return Err(format!(
            "public_key exceeds maximum length ({MAX_SSH_KEY_LEN} bytes)"
        ));
    }
    if !SSH_KEY_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return Err(format!(
            "public_key must start with a valid SSH key type (e.g., ssh-ed25519, ssh-rsa)"
        ));
    }
    // Must have at least type + base64 data
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("public_key must contain type and key data".into());
    }
    Ok(())
}

/// Validate username (alphanumeric, dashes, underscores, dots; max 32 chars).
fn validate_username(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(()); // Will default to "agent"
    }
    if trimmed.len() > MAX_USERNAME_LEN {
        return Err(format!(
            "username exceeds maximum length ({MAX_USERNAME_LEN} chars)"
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("username must be alphanumeric (with dashes, underscores, dots)".into());
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Exec
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecApiRequest {
    pub command: String,
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
            return Err(format!(
                "secret value for '{key}' exceeds max size (64 KB)"
            ));
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
