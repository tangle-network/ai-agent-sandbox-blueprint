//! In-process Firecracker host. There is no separate host-agent.
//!
//! `SANDBOX_FC_WARM_POOL_SIZE=0` (default) is cold boot only. The guest
//! metadata daemon must listen on vsock `MICROVM_GUEST_METADATA_PORT`
//! (default 5555). Empty `image` uses `SANDBOX_FIRECRACKER_DEFAULT_STACK`.
//! `MICROVM_MEM_BACKEND` is read by the primitive, not this module.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use microvm_runtime::{
    GuestMetadataClient, GuestMetadataConfig, NetworkManager, RootfsRegistry, VmNetwork,
    VsockManager,
    adapters::firecracker::{FirecrackerConfig, FirecrackerVmProvider},
    error::VmRuntimeError,
    model::{NetworkInterface, VmSpec, VmStatus, VsockSpec},
    provider::{VmProvider, VmQuery},
};
use rand::RngCore;
use rand::rngs::OsRng;

use crate::error::{Result, SandboxError};
use crate::firecracker_warm::{
    self, TemplateIdentity, WarmClaim, WarmClaimRequest, WarmHost, WarmLineage, WarmOutcome,
    WarmServing, WarmSettings,
};

use crate::firecracker_dnat;

mod context;
mod errors;
mod lifecycle;
mod provisioning;
mod reconcile;
mod types;
mod warm;
mod warm_host;

pub(crate) use context::*;
pub(crate) use errors::*;
pub(crate) use lifecycle::*;
pub(crate) use provisioning::*;
pub(crate) use reconcile::*;
pub(crate) use types::*;
pub(crate) use warm::*;
pub(crate) use warm_host::*;

// Reachable from integration-test crates via `sandbox_runtime::firecracker::…`;
// the pub(crate) glob above would otherwise cap them at crate visibility. Same
// cfg gate as the definitions, so the re-export only exists when they do.
#[cfg(any(test, feature = "test-utils"))]
pub use warm::{reconcile_warm_orphans_for_tests, warm_pool_initialized_for_tests};

#[cfg(test)]
mod tests;
