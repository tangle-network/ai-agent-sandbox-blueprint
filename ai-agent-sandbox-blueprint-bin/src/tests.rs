//! main.rs unit tests.

use super::{WorkflowEntry, validate_chain_vs_host_capacity, workflow_replay_matches_store};
use serde_json::json;

fn active_workflow(id: u64) -> WorkflowEntry {
    WorkflowEntry {
        id,
        name: "workflow-qa".into(),
        workflow_json: "{}".into(),
        trigger_type: "cron".into(),
        trigger_config: "0 * * * * *".into(),
        sandbox_config_json: "{}".into(),
        target_kind: 0,
        target_sandbox_id: String::new(),
        target_service_id: 0,
        active: true,
        next_run_at: None,
        last_run_at: None,
        owner: String::new(),
    }
}

#[test]
fn create_replay_matches_existing_active_workflow() {
    let payload = json!({
        "status": "active",
        "workflowId": 7
    });

    assert!(workflow_replay_matches_store(
        7,
        &payload,
        Some(&active_workflow(7))
    ));
}

#[test]
fn trigger_replay_matches_existing_workflow_even_if_inactive_bit_isnt_rechecked() {
    let payload = json!({
        "status": "active",
        "workflowId": 9,
        "task": {
            "success": true
        }
    });

    assert!(workflow_replay_matches_store(
        12,
        &payload,
        Some(&active_workflow(9))
    ));
}

#[test]
fn canceled_replay_only_matches_when_active_store_entry_is_absent() {
    let payload = json!({
        "status": "canceled",
        "workflowId": 11
    });

    assert!(workflow_replay_matches_store(15, &payload, None));
    assert!(!workflow_replay_matches_store(
        15,
        &payload,
        Some(&active_workflow(11))
    ));
}

// ── chain-vs-host capacity cross-check ─────────────────────────────

#[test]
fn capacity_cross_check_fails_when_chain_exceeds_host() {
    let err = validate_chain_vs_host_capacity(Some("50"), Some("10")).unwrap_err();
    assert!(
        err.contains("OPERATOR_MAX_CAPACITY=50"),
        "names chain value: {err}"
    );
    assert!(
        err.contains("SANDBOX_MAX_COUNT=10"),
        "names host value: {err}"
    );
}

#[test]
fn capacity_cross_check_passes_equal_or_less() {
    assert!(validate_chain_vs_host_capacity(Some("10"), Some("10")).is_ok());
    assert!(validate_chain_vs_host_capacity(Some("5"), Some("10")).is_ok());
}

#[test]
fn capacity_cross_check_passes_uncapped_host() {
    // SANDBOX_MAX_COUNT=0 disables the host cap entirely.
    assert!(validate_chain_vs_host_capacity(Some("500"), Some("0")).is_ok());
}

#[test]
fn capacity_cross_check_requires_both_values() {
    assert!(validate_chain_vs_host_capacity(None, Some("10")).is_ok());
    assert!(validate_chain_vs_host_capacity(Some("50"), None).is_ok());
    assert!(validate_chain_vs_host_capacity(None, None).is_ok());
}

#[test]
fn capacity_cross_check_ignores_unparseable_values() {
    // Parity with the consumers: registration skips a bad
    // OPERATOR_MAX_CAPACITY; the runtime defaults a bad SANDBOX_MAX_COUNT.
    assert!(validate_chain_vs_host_capacity(Some("abc"), Some("10")).is_ok());
    assert!(validate_chain_vs_host_capacity(Some("50"), Some("abc")).is_ok());
}
