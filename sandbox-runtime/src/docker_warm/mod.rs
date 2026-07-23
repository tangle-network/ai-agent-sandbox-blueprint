//! Docker warm-pool: pre-created, pre-started, bootstrapped sidecar containers
//! kept idle and renamed onto the real `sandbox_id` per request.
//!
//! It pre-pays the ~902ms of Docker bring-up (container create ~698ms +
//! container start ~204ms) plus the ~104ms workspace bootstrap exec that the
//! cold path pays on the request. A warm hit does only: rename the container,
//! read back its already-assigned host ports, health-probe, and insert the
//! store record — tens of milliseconds instead of ~1s.
//!
//! The shape mirrors [`crate::firecracker_warm`] 1:1, with one difference
//! Docker forces: **container env is immutable after `docker create`**, so a
//! warm container cannot receive per-request user secrets / base env the way
//! Firecracker injects them over vsock at claim. v1 resolves this honestly with
//! a **shape gate** (see [`serving::DockerWarmServing::shape_gate`]): warm
//! serves ONLY the default shape (default image, default cpu/mem, no user env,
//! matching base env + capabilities, no extra ports, SSH disabled); any other
//! request misses to cold. The auth **token** is still bound at claim because
//! it is a random operator↔sidecar secret (not request-derived): baked at seed,
//! copied verbatim into the store record at claim, no container mutation.
//!
//! ## Admission-control invariant
//!
//! Pool inventory is NOT a live sandbox: it never enters the sandbox store
//! (`sandboxes.json`), so the reaper, GC, `list`, and count cap all stay
//! correct with zero changes — they iterate store records and warm has none. A
//! warm claim itself is admitted exactly like a cold boot: the claim hook sits
//! inside the Docker create arm, which the runtime only reaches AFTER
//! `admit_sandbox_resources` (count cap + host memory budget, under the creation
//! permit) has passed. The pool's own standing RAM footprint is reserved against
//! `SANDBOX_HOST_MEMORY_BUDGET_MB` by [`reserved_host_memory_mb`]
//! (`pool_size × memory_mb`, factor 1 — a Docker warm entry is one container),
//! summed alongside the Firecracker reservation at admission.
//!
//! ## Restart reconciliation
//!
//! Warm containers are identified on the Docker side by the label
//! `tangle.warm-pool=1` and named `sidecar-warm-<seq>` (distinct from the live
//! `sidecar-<uuid>`). [`reconcile_docker_warm_orphans`] lists them by label and
//! reaps every one that is not a live store record (the data-loss guard) BEFORE
//! the first refill — from the pool's lazy init and from
//! [`crate::reaper::reconcile_on_startup`].
//!
//! ## Fail-loud boundaries
//!
//! The only designed fallback is warm-miss → cold boot, and every miss carries
//! a typed [`DockerWarmMiss`] the create path logs. A misconfigured
//! `SANDBOX_DOCKER_WARM_POOL_SIZE`/`SANDBOX_DOCKER_WARM_MEMORY_MB` is a hard
//! error, never a silent disable.
//!
//! Default OFF: `SANDBOX_DOCKER_WARM_POOL_SIZE` unset/0 disables everything —
//! no engine init, no seeding, no memory reservation — so Firecracker-only and
//! plain-Docker hosts pay nothing.

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
mod reconcile;
mod serving;
mod types;

pub(crate) use config::*;
pub(crate) use reconcile::*;
pub(crate) use serving::*;
pub(crate) use types::*;

#[cfg(test)]
mod tests;
