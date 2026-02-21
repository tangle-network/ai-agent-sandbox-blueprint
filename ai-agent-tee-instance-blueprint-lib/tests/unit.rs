//! Unit tests for the AI Agent TEE Instance Blueprint library.
//!
//! No Docker, no wiremock. Pure unit tests for the TEE lib's own code:
//! re-exports, TeeBackend global state, router construction, and
//! core provision/deprovision signatures.

use std::sync::Once;

static INIT: Once = Once::new();

fn init() {
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("tee-instance-bp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        // SAFETY: tests run single-threaded during init; no concurrent env reads.
        unsafe {
            std::env::set_var("BLUEPRINT_STATE_DIR", dir.to_str().unwrap());
            std::env::set_var("SIDECAR_IMAGE", "nginx:alpine");
            std::env::set_var("SIDECAR_PULL_IMAGE", "true");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            std::env::set_var("REQUEST_TIMEOUT_SECS", "10");
        }
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// RE-EXPORT VERIFICATION
// ═══════════════════════════════════════════════════════════════════════════

mod re_export_tests {
    use ai_agent_tee_instance_blueprint_lib::*;

    #[test]
    fn job_constants_accessible() {
        assert_eq!(JOB_PROVISION, 5);
        assert_eq!(JOB_DEPROVISION, 6);
    }

    #[test]
    fn abi_types_roundtrip_through_tee_lib() {
        use blueprint_sdk::alloy::sol_types::SolValue;

        let output = ProvisionOutput {
            sandbox_id: "tee-sb-123".to_string(),
            sidecar_url: "http://tee-sidecar:8080".to_string(),
            ssh_port: 2222,
            tee_attestation_json: r#"{"tee_type":"phala"}"#.to_string(),
            tee_public_key_json: String::new(),
        };

        let encoded = output.abi_encode();
        let decoded = ProvisionOutput::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.sandbox_id, "tee-sb-123");
        assert_eq!(decoded.sidecar_url, "http://tee-sidecar:8080");
        assert_eq!(decoded.ssh_port, 2222);
        assert_eq!(decoded.tee_attestation_json, r#"{"tee_type":"phala"}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEE ROUTER CONSTRUCTION
// ═══════════════════════════════════════════════════════════════════════════

mod router_tests {
    use ai_agent_tee_instance_blueprint_lib::tee_router;

    #[test]
    fn tee_router_builds_successfully() {
        // Verifies the router can be constructed without panicking.
        // This is both a compile-time and runtime check that all job handlers
        // are correctly wired.
        let _router = tee_router();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INSTANCE STATE (via TEE lib re-exports)
// ═══════════════════════════════════════════════════════════════════════════

mod instance_state_tests {
    use super::*;
    use ai_agent_tee_instance_blueprint_lib::*;

    #[test]
    fn instance_store_initializes_through_tee_lib() {
        init();
        let store = instance_store();
        assert!(store.is_ok());
    }

    #[test]
    fn get_instance_returns_none_when_empty() {
        init();
        let _ = clear_instance_sandbox();
        let result = get_instance_sandbox().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn require_instance_fails_when_not_provisioned() {
        init();
        let _ = clear_instance_sandbox();
        let result = require_instance_sandbox();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not provisioned"));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEE BACKEND GLOBAL STATE
// ═══════════════════════════════════════════════════════════════════════════
//
// We can't test init_tee_backend/tee_backend directly in unit tests because
// they use a process-global OnceCell — calling init in one test would affect
// all others. Instead, we verify the API exists and is callable by checking
// that tee_backend() panics when NOT initialized (the expected behavior).

mod tee_backend_tests {
    #[test]
    #[should_panic(expected = "TEE backend not initialized")]
    fn tee_backend_panics_when_not_initialized() {
        // This test MUST run in isolation. Because we haven't called
        // init_tee_backend(), accessing it should panic with the expected message.
        // Note: if another test in this binary calls init_tee_backend(), this
        // test may fail — that's by design (global state).
        let _ = ai_agent_tee_instance_blueprint_lib::tee_backend();
    }
}
