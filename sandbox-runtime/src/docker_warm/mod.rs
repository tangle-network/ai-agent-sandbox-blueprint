//! Docker warm pool. Default off (`SANDBOX_DOCKER_WARM_POOL_SIZE` unset or 0).
//!
//! Container env is immutable after create, so warm serves only the default
//! shape; anything else misses to cold. The auth token is baked at seed.
//! Pool containers are not store records. Admission still runs first, and the
//! host budget reserves `pool_size × memory_mb`. Reap only names that still
//! start with `sidecar-warm-`; a claim renames to `sidecar-<id>` first, so a
//! bad store cannot classify a live sandbox as an orphan.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    ListContainersOptions, RemoveContainerOptions, RenameContainerOptions,
};
use docktopus::container::Container;

use crate::error::{Result, SandboxError};
use crate::runtime::{CreateSandboxParams, SidecarRuntimeConfig};

mod config;
mod host;
mod reconcile;
mod serving;
mod types;

pub(crate) use config::*;
pub(crate) use host::*;
pub(crate) use reconcile::*;
pub(crate) use serving::*;
pub(crate) use types::*;

#[cfg(test)]
mod tests;
