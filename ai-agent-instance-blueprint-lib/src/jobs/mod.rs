pub mod exec;
pub mod provision;
pub mod snapshot;
pub mod ssh;
pub mod workflow;

pub(crate) fn caller_hex(caller: &[u8; 20]) -> String {
    let addr = blueprint_sdk::alloy::primitives::Address::from_slice(caller);
    format!("{addr:#x}")
}
