//! Reaper and garbage collection for sandbox lifecycle enforcement.
//!
//! - `reaper_tick()`: stops idle sandboxes, deletes expired ones
//! - `gc_tick()`: removes stopped sandboxes past retention period
//! - `reconcile_on_startup()`: syncs store state with Docker reality

use crate::metrics::metrics;
use crate::runtime::{
    SandboxState, SidecarRuntimeConfig, commit_container, delete_sidecar, docker_builder,
    record_uses_firecracker, refresh_docker_sandbox_endpoint, remove_snapshot_image, sandboxes,
    stop_sidecar, supports_docker_endpoint_refresh,
};
use blueprint_sdk::{error, info};
use docktopus::bollard::container::InspectContainerOptions;

mod gc;
mod reconcile;
mod snapshot;
mod tick;

pub use gc::gc_tick;
pub use reconcile::reconcile_on_startup;
pub(crate) use snapshot::*;
pub use tick::reaper_tick;

#[cfg(test)]
mod tests;
