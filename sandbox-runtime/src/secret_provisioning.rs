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
use crate::runtime::{get_sandbox_by_id, recreate_sidecar_with_env, sandboxes, SandboxRecord};

/// Inject secrets into a sandbox by recreating it with merged environment.
///
/// Merges `secret_env` into the sandbox's existing `env_json`. Existing keys
/// are overwritten by `secret_env` values. Sets `secrets_configured = true`
/// on the sandbox record.
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn inject_secrets(
    sandbox_id: &str,
    secret_env: Map<String, Value>,
) -> Result<SandboxRecord> {
    let record = get_sandbox_by_id(sandbox_id)?;

    // Merge existing env with new secrets
    let merged = merge_env(&record.env_json, &secret_env);

    let new_record = recreate_sidecar_with_env(sandbox_id, &merged, None).await?;

    // Mark secrets as configured on the new record
    sandboxes()?.update(&new_record.id, |r| {
        r.secrets_configured = true;
    })?;

    // Return the updated record
    Ok(get_sandbox_by_id(&new_record.id)?)
}

/// Remove all user-injected secrets from a sandbox by recreating it with
/// only the base environment (empty env_json). Sets `secrets_configured = false`
/// on the sandbox record.
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn wipe_secrets(sandbox_id: &str) -> Result<SandboxRecord> {
    let new_record = recreate_sidecar_with_env(sandbox_id, "{}", None).await?;

    // Mark secrets as wiped on the new record
    sandboxes()?.update(&new_record.id, |r| {
        r.secrets_configured = false;
    })?;

    Ok(get_sandbox_by_id(&new_record.id)?)
}

/// Merge existing env_json with additional secret key-value pairs.
fn merge_env(existing_env_json: &str, extra: &Map<String, Value>) -> String {
    let mut env: Map<String, Value> = if existing_env_json.trim().is_empty() {
        Map::new()
    } else {
        serde_json::from_str(existing_env_json).unwrap_or_default()
    };

    env.extend(extra.clone());
    serde_json::to_string(&env).unwrap_or_default()
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
    use super::*;

    #[test]
    fn merge_env_empty_base() {
        let extra = serde_json::from_str::<Map<String, Value>>(
            r#"{"API_KEY": "secret123"}"#,
        )
        .unwrap();
        let result = merge_env("", &extra);
        let parsed: Map<String, Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["API_KEY"], "secret123");
    }

    #[test]
    fn merge_env_with_existing() {
        let extra = serde_json::from_str::<Map<String, Value>>(
            r#"{"API_KEY": "secret123", "BASE": "override"}"#,
        )
        .unwrap();
        let result = merge_env(r#"{"BASE": "original", "OTHER": "keep"}"#, &extra);
        let parsed: Map<String, Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["API_KEY"], "secret123");
        assert_eq!(parsed["BASE"], "override");
        assert_eq!(parsed["OTHER"], "keep");
    }

    #[test]
    fn merge_env_empty_extra() {
        let extra = Map::new();
        let result = merge_env(r#"{"FOO": "bar"}"#, &extra);
        let parsed: Map<String, Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["FOO"], "bar");
    }
}
