//! Firecracker microVM lifecycle wrapper.
//!
//! Thin adapter around the [`microvm-runtime`] crate's in-process Firecracker
//! driver. The operator binary **is** the Firecracker host — there is no
//! separate "host-agent" service; this module talks directly to the VMM over
//! its unix socket via the primitive.
//!
//! ## Wired today (`microvm-runtime 0.4.0-alpha.2` + `microvm-warm-pool 0.1.0-alpha.2`)
//!
//! - VM create / start / stop / destroy lifecycle.
//! - **Warm-pool snapshot serving** via [`crate::firecracker_warm`], gated
//!   by `SANDBOX_FC_WARM_POOL_SIZE` (0 = off, the default). When enabled,
//!   dedicated template VMs (never tenant VMs) are cold-booted, paused, and
//!   snapshotted; pre-restored entries wait in a [`microvm_warm_pool`] pool
//!   and are handed off to creates via `rename_vm` + resume. Misses fall
//!   back to cold boot with a logged, typed reason — the one designed
//!   fallback in this module.
//! - **Snapshot-restore memory backend**: `MICROVM_MEM_BACKEND=file|uffd`
//!   is read by the primitive itself (`FirecrackerConfig::from_env`);
//!   `uffd` restores guest memory through a userfaultfd handler (lazy CoW
//!   paging from the snapshot's mem file) instead of a full file load.
//! - **Per-VM TAP / bridge / NAT** via [`NetworkManager`]. The host bridge,
//!   per-VM TAP, and gateway are set up before `create_vm_with_spec`; the
//!   resulting [`VmNetwork`] is recorded so the host-reachable sidecar URL
//!   can be built from the guest IP.
//! - **Per-VM vsock CID + UDS** via [`VsockManager`]. Provisioned pre-boot;
//!   parent dir guaranteed to exist before any `/snapshot/load`.
//! - **Per-VM iptables DNAT** in [`firecracker_dnat`]. Each
//!   `metadata_json.ports` entry installs a PREROUTING DNAT rule mapping
//!   `host_port → guest_ip:container_port`. Rules are tracked per VM and
//!   released on delete.
//! - **Per-VM resource overrides**: `cpu_cores` and `memory_mb` from the
//!   create request flow into `VmSpec` (clamped to FC's u8 / u32 ranges).
//! - **Per-VM disk sizing**: when `req.disk_gb > 0` the request's chosen
//!   stack is cloned through [`RootfsRegistry::clone_for_vm_with_size`] and
//!   the resulting per-VM ext4 image is wired into [`VmSpec::rootfs`]. The
//!   default stack name comes from `SANDBOX_FIRECRACKER_DEFAULT_STACK` when
//!   `req.image` is empty; when both are absent the workspace default
//!   rootfs path baked into the provider is reused untouched.
//! - **Per-VM environment + sidecar auth token injection** via the guest
//!   metadata service ([`GuestMetadataClient`]). Post-boot, the host opens
//!   the per-VM vsock UDS and pushes the full `req.env` map plus a freshly
//!   minted 32-byte sidecar auth token into the guest. The token is also
//!   returned to the caller so the runtime layer can stamp it onto the
//!   sandbox record.
//! - **Host-reachable sidecar endpoint URL** computed from the composer-
//!   assigned guest IP and the sidecar port (`SIDECAR_PORT` env, default
//!   8080).
//! - Status reporting for the reaper reconcile loop
//!   (`FirecrackerContainerStatus::{Missing,Running,Stopped}`).
//! - Provider initialization probe used by the operator API health check.
//!
//! ## Operator prerequisites
//!
//! - A guest-side metadata daemon listening on vsock port
//!   `MICROVM_GUEST_METADATA_PORT` (default `5555`) baked into the rootfs.
//!   The reference implementation ships at
//!   `microvm-runtime/examples/guest_metadata_daemon.rs`; operators should
//!   install it as a systemd unit (or equivalent) inside their stack image.
//! - Stack templates under `MICROVM_ROOTFS_TEMPLATE_DIR` with per-VM
//!   clones written to `MICROVM_ROOTFS_CLONES_DIR`. The default stack name
//!   used when the create request leaves `image` empty is configured via
//!   `SANDBOX_FIRECRACKER_DEFAULT_STACK`.

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
