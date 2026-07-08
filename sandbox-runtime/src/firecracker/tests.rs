//! firecracker.rs unit tests.

use super::*;

#[cfg(test)]
mod firecracker_cases {
    use super::*;

    #[test]
    fn reclaim_clone_predicate_is_data_loss_safe() {
        use std::collections::HashSet;
        let referenced: HashSet<String> = ["fcwarm-g0-tpl".to_string()].into_iter().collect();
        // A live sandbox's own-id (non-warm) clone is NEVER reclaimed — the
        // data-loss guard: removing it would destroy a running workspace.
        assert!(!should_reclaim_clone("sandbox-live-1a2b3c", &referenced));
        assert!(!should_reclaim_clone("a1b2c3d4-uuid", &referenced));
        // A warm template a live sandbox's lineage still references is kept.
        assert!(!should_reclaim_clone("fcwarm-g0-tpl", &referenced));
        // A warm-template clone no live lineage references is reclaimed.
        assert!(should_reclaim_clone("fcwarm-g1-tpl", &referenced));
        assert!(should_reclaim_clone(
            "warm-node20-1_0_0-seed",
            &HashSet::new()
        ));
    }

    #[test]
    fn map_vm_error_translates_not_found_to_sandbox_not_found() {
        // Regression: `stop`/`delete` rely on `SandboxError::NotFound` being
        // the variant they pattern-match for idempotent treatment. Pinning
        // the mapping prevents a silent semantic drift.
        let err = map_vm_error("test", "vm-1", VmRuntimeError::VmNotFound("vm-1".into()));
        assert!(matches!(err, SandboxError::NotFound(_)), "got {err}");
    }

    #[test]
    fn map_vm_error_translates_invalid_transition_to_validation() {
        let err = map_vm_error(
            "test",
            "vm-1",
            VmRuntimeError::InvalidTransition {
                vm_id: "vm-1".into(),
                from: "created".into(),
                to: "running",
            },
        );
        assert!(matches!(err, SandboxError::Validation(_)), "got {err}");
    }

    #[test]
    fn map_vm_error_translates_guest_metadata_to_unavailable() {
        // Regression: when the guest daemon is unreachable or rejects a
        // request, we surface it as `Unavailable` (operator can install the
        // daemon, restart the VM, etc.) — never as `Unsupported`, which
        // would falsely claim the feature is unimplemented.
        let err = map_vm_error(
            "test",
            "vm-1",
            VmRuntimeError::GuestMetadata("daemon did not reply".into()),
        );
        assert!(matches!(err, SandboxError::Unavailable(_)), "got {err:?}");
        assert!(err.to_string().contains("daemon did not reply"));
    }

    #[test]
    fn mint_sidecar_token_is_64_hex_chars_and_unique() {
        let a = mint_sidecar_token();
        let b = mint_sidecar_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn attachments_roundtrip_is_take_once() {
        // Regression: a second `delete` after the first must not see the
        // attachments again (otherwise we'd double-release DNAT rules,
        // which is fine, but also issue a misleading WARN log).
        let vm_id = "vm-attach-roundtrip";
        // Ensure no leftover state from a previous test run.
        let _ = take_attachments(vm_id);
        record_attachments(vm_id, VmAttachments::cold(2, true));
        let first = take_attachments(vm_id).expect("first take returns recorded value");
        assert_eq!(first.dnat_rule_count, 2);
        assert!(first.rootfs_cloned);
        assert!(take_attachments(vm_id).is_none());
    }

    #[test]
    fn default_stack_name_round_trips_through_env() {
        // Use a hermetic guard: capture the prior value, set, observe, restore.
        // SAFETY: the surrounding tests acquire a process-wide env mutex in
        // `lib.rs` (TEST_ENV_GUARD); this unit test is the only place in
        // this module that mutates env, so a localised lock would buy
        // nothing — the helper just round-trips and restores.
        let prior = std::env::var("SANDBOX_FIRECRACKER_DEFAULT_STACK").ok();
        unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", "node-20") };
        assert_eq!(default_stack_name().as_deref(), Some("node-20"));
        unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", "   ") };
        assert!(
            default_stack_name().is_none(),
            "empty/whitespace must be None"
        );
        match prior {
            Some(v) => unsafe { std::env::set_var("SANDBOX_FIRECRACKER_DEFAULT_STACK", v) },
            None => unsafe { std::env::remove_var("SANDBOX_FIRECRACKER_DEFAULT_STACK") },
        }
    }
}
