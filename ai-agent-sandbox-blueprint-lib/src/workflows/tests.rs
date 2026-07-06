use super::*;
use blueprint_sdk::alloy::dyn_abi::DynSolValue;
use blueprint_sdk::alloy::primitives::U256;

#[test]
fn dyn_string_extracts() {
    let val = DynSolValue::String("hello".into());
    assert_eq!(dyn_string(&val).unwrap(), "hello");
}

#[test]
fn dyn_string_rejects_non_string() {
    let val = DynSolValue::Bool(true);
    assert!(dyn_string(&val).is_err());
}

#[test]
fn dyn_bool_extracts() {
    assert!(dyn_bool(&DynSolValue::Bool(true)).unwrap());
    assert!(!dyn_bool(&DynSolValue::Bool(false)).unwrap());
}

#[test]
fn dyn_bool_rejects_non_bool() {
    let val = DynSolValue::String("yes".into());
    assert!(dyn_bool(&val).is_err());
}

#[test]
fn dyn_u64_extracts() {
    let val = DynSolValue::Uint(U256::from(42u64), 64);
    assert_eq!(dyn_u64(&val).unwrap(), 42);
}

#[test]
fn workflow_registry_abi_parses() {
    let _: blueprint_sdk::alloy::json_abi::JsonAbi =
        serde_json::from_str(WORKFLOW_REGISTRY_ABI).expect("workflow registry ABI should parse");
}

#[test]
fn dyn_u64_overflow() {
    let val = DynSolValue::Uint(U256::MAX, 256);
    assert!(dyn_u64(&val).is_err());
}

#[test]
fn parse_workflow_ids_empty() {
    let input = vec![DynSolValue::Array(vec![])];
    assert_eq!(parse_workflow_ids(input).unwrap(), Vec::<u64>::new());
}

#[test]
fn parse_workflow_ids_multiple() {
    let input = vec![DynSolValue::Array(vec![
        DynSolValue::Uint(U256::from(1u64), 64),
        DynSolValue::Uint(U256::from(2u64), 64),
        DynSolValue::Uint(U256::from(3u64), 64),
    ])];
    assert_eq!(parse_workflow_ids(input).unwrap(), vec![1, 2, 3]);
}

#[test]
fn workflow_run_guard_tracks_running_state() {
    let workflow_id = u64::MAX - 41;
    assert!(!is_workflow_running(workflow_id));

    let guard = acquire_workflow_run(workflow_id).unwrap();
    assert!(is_workflow_running(workflow_id));
    assert!(acquire_workflow_run(workflow_id).is_err());

    drop(guard);
    assert!(!is_workflow_running(workflow_id));
}
