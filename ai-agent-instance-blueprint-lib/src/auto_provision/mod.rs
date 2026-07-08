//! Auto-provision: reads service config from BSM on-chain, provisions sandbox
//! automatically on startup.
//!
//! Flow:
//! 1. On startup, check if already provisioned (`get_instance_sandbox()`)
//! 2. If not, poll `getServiceConfig(serviceId)` from BSM via RPC
//! 3. When config available, decode as `ProvisionRequest`, call `provision_core()`
//! 4. Store sandbox record via `set_instance_sandbox()`
//! 5. Report provision directly to manager contract (`reportProvisioned`)
//!

use blueprint_sdk::alloy::primitives::Address;
use blueprint_sdk::alloy::providers::ProviderBuilder;
use blueprint_sdk::alloy::sol_types::SolValue;
use blueprint_sdk::{info, warn};
use std::time::Duration;

use crate::tee::TeeBackend;
use crate::{
    IBsmRead, LegacyProvisionRequest, ProvisionRequest, ProvisionRequestV1, clear_instance_sandbox,
    ensure_local_provision_reported, get_instance_sandbox, mark_pending_provision_report,
    provision_core, report_local_provision, set_instance_sandbox,
};

mod chain_read;
mod config;
mod record;
mod runner;

#[cfg(test)]
mod tests;

pub use chain_read::*;
pub use config::*;
pub(crate) use record::*;
pub use runner::*;
