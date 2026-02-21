//! Serde-based request/response types for the operator HTTP API.
//!
//! These parallel the `sol!` ABI types in `instance_types.rs` but use
//! serde for JSON serialization — needed because `sol!` structs don't
//! implement `Serialize`/`Deserialize`.

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize)]
pub struct ExecApiResponse {
    pub exit_code: u32,
    pub stdout: String,
    pub stderr: String,
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

#[derive(Debug, Deserialize)]
pub struct SshRevokeApiRequest {
    #[serde(default = "default_ssh_username")]
    pub username: String,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct SshApiResponse {
    pub success: bool,
    pub result: serde_json::Value,
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
