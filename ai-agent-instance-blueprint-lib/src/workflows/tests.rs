#[test]
fn workflow_registry_abi_parses() {
    let _: blueprint_sdk::alloy::json_abi::JsonAbi =
        serde_json::from_str(super::WORKFLOW_REGISTRY_ABI)
            .expect("workflow registry ABI should parse");
}

#[test]
fn workflow_run_guard_tracks_running_state() {
    let workflow_id = u64::MAX - 29;
    assert!(!super::is_workflow_running(workflow_id));

    let guard = super::acquire_workflow_run(workflow_id).unwrap();
    assert!(super::is_workflow_running(workflow_id));
    assert!(super::acquire_workflow_run(workflow_id).is_err());

    drop(guard);
    assert!(!super::is_workflow_running(workflow_id));
}
