//! Two-phase secret provisioning for sandboxes.
//!
//! Phase 1 (on-chain): Sandbox is created with base configuration only.
//! No secrets appear in transaction calldata.
//!
//! Phase 2 (off-chain): The sandbox owner sends secrets via a signed HTTP
//! request to the operator API. The operator recreates the sidecar container
//! with the full environment (base config + secrets).
//!
//! This pattern ensures that API keys, private keys, and other sensitive
//! values never touch the blockchain.

use serde_json::{Map, Value};

use crate::error::{Result, SandboxError};
use crate::runtime::{get_sandbox_by_id, recreate_sidecar_with_env, SandboxRecord};

/// Inject user secrets into a sandbox by recreating it with merged environment.
///
/// The sandbox's `base_env_json` is preserved. The provided `secret_env` is
/// stored as `user_env_json` and merged on top of the base at container creation.
/// User values override base values when keys collide.
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn inject_secrets(
    sandbox_id: &str,
    secret_env: Map<String, Value>,
) -> Result<SandboxRecord> {
    let user_env_json = serde_json::to_string(&secret_env)
        .map_err(|e| SandboxError::Validation(format!("Invalid secret env: {e}")))?;

    let new_record = recreate_sidecar_with_env(sandbox_id, &user_env_json, None).await?;
    Ok(new_record)
}

/// Remove all user-injected secrets from a sandbox by recreating it with
/// only the base environment. The `base_env_json` is preserved.
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn wipe_secrets(sandbox_id: &str) -> Result<SandboxRecord> {
    let new_record = recreate_sidecar_with_env(sandbox_id, "", None).await?;
    Ok(new_record)
}

/// Validate that the caller (identified by session address) owns the sandbox.
pub fn validate_secret_access(
    sandbox_id: &str,
    caller_address: &str,
) -> Result<SandboxRecord> {
    let record = get_sandbox_by_id(sandbox_id)?;
    if record.owner.is_empty() || record.owner.eq_ignore_ascii_case(caller_address) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Address {caller_address} does not own sandbox '{sandbox_id}'"
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::merge_env_json;

    #[test]
    fn merge_env_empty_base() {
        let result = merge_env_json("", r#"{"API_KEY": "secret123"}"#);
        assert_eq!(result, r#"{"API_KEY":"secret123"}"#);
    }

    #[test]
    fn merge_env_user_overrides_base() {
        let result = merge_env_json(
            r#"{"BASE": "original", "OTHER": "keep"}"#,
            r#"{"API_KEY": "secret123", "BASE": "override"}"#,
        );
        let parsed: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["API_KEY"], "secret123");
        assert_eq!(parsed["BASE"], "override");
        assert_eq!(parsed["OTHER"], "keep");
    }

    #[test]
    fn merge_env_empty_user_returns_base() {
        let result = merge_env_json(r#"{"FOO": "bar"}"#, "");
        assert_eq!(result, r#"{"FOO": "bar"}"#);
    }

    #[test]
    fn merge_env_empty_object_user_returns_base() {
        let result = merge_env_json(r#"{"FOO": "bar"}"#, "{}");
        assert_eq!(result, r#"{"FOO": "bar"}"#);
    }
}
