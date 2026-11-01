//! Firecracker warm-pool snapshot serving.
//!
//! Pool VMs are not store records. A claim is admitted like a cold boot, after
//! `admit_sandbox_resources`. The host budget reserves `pool_size × 2 × mem`
//! even under uffd, because a resumed guest can fault the entry fully in.
//! One generation serves one claim (template TAP stays held while paused, so
//! the entry restores onto a rider TAP). Warm-miss falls back to cold boot
//! with a typed reason. Restart reaps `fcwarm-*` and `warm-*` before the
//! first seed; `pkill -f fcwarm-` misses the restored entries.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use microvm_runtime::{
    model::{NetworkInterface, SnapshotRef, VmSpec, VmStatus},
    provider::{VmProvider, VmQuery},
};
#[cfg(test)]
use microvm_warm_pool::WarmPoolMetrics;
use microvm_warm_pool::{EntryValidator, StackKey, ValidationResult, WarmPool, WarmPoolConfig};

use crate::error::{Result, SandboxError};

mod config;
mod host;
mod serving;
mod types;

pub(crate) use config::*;
pub(crate) use host::*;
pub(crate) use serving::*;
pub(crate) use types::*;

#[cfg(test)]
mod tests;
