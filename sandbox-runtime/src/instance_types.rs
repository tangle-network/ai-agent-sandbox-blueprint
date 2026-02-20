//! Shared ABI types for instance-scoped blueprints (exec, prompt, task).
//!
//! These are used by both `ai-agent-instance-blueprint-lib` and
//! `trading-instance-blueprint-lib` (and their TEE variants). Defined once
//! here to avoid duplicating identical `sol!` blocks across workspaces.

use alloy::sol;

sol! {
    // ── Exec (instance-scoped — no sidecar_url/token) ───────────────────

    struct InstanceExecRequest {
        string command;
        string cwd;
        string env_json;
        uint64 timeout_ms;
    }

    struct InstanceExecResponse {
        uint32 exit_code;
        string stdout;
        string stderr;
    }

    // ── Prompt (instance-scoped — no sidecar_url/token) ─────────────────

    struct InstancePromptRequest {
        string message;
        string session_id;
        string model;
        string context_json;
        uint64 timeout_ms;
    }

    struct InstancePromptResponse {
        bool success;
        string response;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
    }

    // ── Task (instance-scoped — no sidecar_url/token) ───────────────────

    struct InstanceTaskRequest {
        string prompt;
        string session_id;
        uint64 max_turns;
        string model;
        string context_json;
        uint64 timeout_ms;
    }

    struct InstanceTaskResponse {
        bool success;
        string result;
        string error;
        string trace_id;
        uint64 duration_ms;
        uint32 input_tokens;
        uint32 output_tokens;
        string session_id;
    }
}
