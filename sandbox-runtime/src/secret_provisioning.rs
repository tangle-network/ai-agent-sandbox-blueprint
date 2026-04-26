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
use crate::runtime::{SandboxRecord, get_sandbox_by_id, recreate_sidecar_with_env};

/// Inject user secrets into a sandbox by recreating it with merged environment.
///
/// The sandbox's `base_env_json` is preserved. The provided `secret_env` is
/// stored as `user_env_json` and merged on top of the base at container creation.
/// User values override base values when keys collide.
///
/// **TEE restriction:** This function is not supported for TEE sandboxes because
/// recreation would invalidate the attestation, break sealed secrets, and orphan
/// the on-chain deployment ID. TEE sandboxes should use the sealed-secrets API
/// (`POST /tee/sealed-secrets`) instead.
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn inject_secrets(
    sandbox_id: &str,
    secret_env: Map<String, Value>,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    let user_env_json = serde_json::to_string(&secret_env)
        .map_err(|e| SandboxError::Validation(format!("Invalid secret env: {e}")))?;

    let new_record = recreate_sidecar_with_env(sandbox_id, &user_env_json, tee).await?;
    Ok(new_record)
}

/// Remove all user-injected secrets from a sandbox by recreating it with
/// only the base environment. The `base_env_json` is preserved.
///
/// **TEE restriction:** Not supported for TEE sandboxes — see [`inject_secrets`].
///
/// Returns the new `SandboxRecord` for the recreated sandbox.
pub async fn wipe_secrets(
    sandbox_id: &str,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    let new_record = recreate_sidecar_with_env(sandbox_id, "", tee).await?;
    Ok(new_record)
}

/// Validate that the caller (identified by session address) owns the sandbox.
pub fn validate_secret_access(sandbox_id: &str, caller_address: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_id(sandbox_id)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth("Sandbox has no owner configured".into()));
    }
    if record.owner.eq_ignore_ascii_case(caller_address) {
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

    // ── Phase 1E: Secret Provisioning Identity Immutability Tests ────────

    #[test]
    fn merge_env_preserves_base_keys() {
        // After merge then clear (empty user env), base keys must survive
        let base = r#"{"BASE_KEY": "base_value", "SHARED": "original"}"#;
        let user = r#"{"SECRET": "s3cr3t", "SHARED": "override"}"#;

        // Step 1: merge user on top of base
        let merged = merge_env_json(base, user);
        let parsed: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed["BASE_KEY"], "base_value");
        assert_eq!(parsed["SECRET"], "s3cr3t");
        assert_eq!(parsed["SHARED"], "override");

        // Step 2: clear user secrets (merge with empty)
        let cleared = merge_env_json(base, "");
        let parsed_cleared: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&cleared).unwrap();
        assert_eq!(
            parsed_cleared["BASE_KEY"], "base_value",
            "base key must survive inject/wipe cycle"
        );
        assert_eq!(
            parsed_cleared["SHARED"], "original",
            "base value must revert after wipe"
        );
        assert!(
            !parsed_cleared.contains_key("SECRET"),
            "user secret must be gone after wipe"
        );
    }

    #[test]
    fn validate_secret_access_same_id_after_wipe() {
        // Verifies that sandbox_id is stable across inject/wipe by testing
        // the validate_secret_access function's ID-based lookup. Since we
        // can't run full sidecar recreation in unit tests, we verify the
        // ID-based access function works correctly.
        use crate::runtime::{SandboxRecord, SandboxState, sandboxes, seal_record};

        // Ensure store is initialized
        let dir = std::env::temp_dir().join(format!("secret-prov-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", &dir) };

        let sandbox_id = "secret-id-stable-1";
        let owner = "0xSECRETOWNER00000000000000000000000001";
        let mut record = SandboxRecord {
            id: sandbox_id.to_string(),
            container_id: "ctr-secret-1".to_string(),
            sidecar_url: "http://localhost:9999".to_string(),
            sidecar_port: 9999,
            ssh_port: None,
            token: "test".into(),
            created_at: 1_700_000_000,
            cpu_cores: 1,
            memory_mb: 1024,
            state: SandboxState::Running,
            idle_timeout_seconds: 1800,
            max_lifetime_seconds: 86400,
            last_activity_at: 1_700_000_000,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: "test:latest".into(),
            base_env_json: r#"{"BASE":"val"}"#.into(),
            user_env_json: String::new(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: "test".into(),
            agent_identifier: String::new(),
            metadata_json: "{}".into(),
            disk_gb: 10,
            stack: String::new(),
            owner: owner.to_string(),
            service_id: None,
            tee_config: None,
            extra_ports: std::collections::HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
            capabilities_json: String::new(),
        };
        seal_record(&mut record).unwrap();
        sandboxes()
            .unwrap()
            .insert(sandbox_id.to_string(), record)
            .unwrap();

        // Validate access returns the same sandbox_id
        let accessed = crate::secret_provisioning::validate_secret_access(sandbox_id, owner)
            .expect("should validate");
        assert_eq!(
            accessed.id, sandbox_id,
            "sandbox_id must be stable across access validation"
        );

        // Simulate wipe: update user_env_json to empty
        sandboxes()
            .unwrap()
            .update(sandbox_id, |r| {
                r.user_env_json = String::new();
            })
            .unwrap();

        // Re-validate: same sandbox_id
        let accessed_after = crate::secret_provisioning::validate_secret_access(sandbox_id, owner)
            .expect("should still validate after wipe");
        assert_eq!(
            accessed_after.id, sandbox_id,
            "sandbox_id must be immutable across secrets inject/wipe"
        );
    }
}
