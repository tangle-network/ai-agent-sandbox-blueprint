//! Firecracker warm-pool snapshot serving.
//!
//! Keeps `SANDBOX_FC_WARM_POOL_SIZE` *generations* warm. A generation is a
//! self-contained unit of pre-provisioned identity:
//!
//! - a **template VM** (`fcwarm-g<N>-tpl`): cold-booted once on the
//!   operator's configured base stack with its own TAP / vsock / rootfs,
//!   paused, and snapshotted (`golden`). It is kept paused afterwards
//!   because the snapshot artifacts live in its state dir — destroying it
//!   would delete them (`microvm-runtime` layout).
//! - a **rider TAP** (`fcwarm-g<N>-rider`): the host interface the pooled
//!   entry restores onto via `SnapshotRef::network_overrides` (the template
//!   still holds its own TAP while paused, so the entry cannot reuse it).
//! - a [`WarmPool`] bucket of depth 1 holding one **pre-restored, paused
//!   entry** ready for handoff.
//!
//! A claim is `acquire → rename_vm(entry, sandbox_id) → start_vm` — the
//! handoff flow `microvm-runtime 0.4.0-alpha.2` implements `rename_vm` for.
//! The claimed VM inherits the generation's identity: the template's guest
//! IP (baked into the snapshot's memory image), the template's vsock UDS +
//! CID (recorded in the vmstate), the template's rootfs backing file, and
//! the rider TAP. Because that identity is single-occupancy, a generation
//! serves exactly one claim: on claim the bucket is unregistered, the paused
//! template is destroyed (releasing its memory and its TAP), and a fresh
//! generation is seeded on the next create. Idle-time entry evictions
//! (age / failed validation) do NOT retire the generation — the refill
//! thread restores a replacement from the still-alive template snapshot,
//! reusing the rider TAP sequentially.
//!
//! ## Admission-control invariant
//!
//! Pool inventory (templates + pre-restored entries) is NOT a live sandbox:
//! it never enters the sandbox store. A warm claim itself is admitted exactly
//! like a cold boot — the claim runs inside `firecracker::create_and_start`,
//! which the runtime layer only calls AFTER `admit_sandbox_resources` +
//! `enforce_sandbox_count_limit` have passed under the creation permit.
//!
//! The pool's own standing footprint is reserved against the host memory
//! budget by [`reserved_host_memory_mb`]: `admit_sandbox_resources` adds
//! `pool_size × 2 × mem_size_mib` (paused template + pre-restored entry per
//! generation, the `file`-backend worst case) to committed memory, so
//! `SANDBOX_HOST_MEMORY_BUDGET_MB` accounts for pool inventory even though it
//! never enters the store. The factor stays 2 under `MICROVM_MEM_BACKEND=uffd`
//! (a resumed guest can fault its entry pages fully in) to keep the guard
//! fail-closed.
//!
//! ## Fail-loud boundaries
//!
//! The single designed fallback is warm-miss → cold boot, and every miss
//! carries a typed [`WarmMiss`] reason that the create path logs. Seeding
//! failures are logged loudly and retried on subsequent creates
//! ([`WarmServing::ensure_seeding`]); a misconfigured
//! `SANDBOX_FC_WARM_POOL_SIZE` is a hard error, never a silent disable.
//!
//! ## Restart reconciliation
//!
//! Pool VMs are process-local: after an operator restart the fresh process's
//! provider maps are empty, so prior `fcwarm-*` templates and `warm-*` pool
//! entries would keep running (holding guest memory) with no sandbox record.
//! `firecracker::reconcile_warm_orphans` reaps them from `/proc` before the
//! first seed — enumerating warm-prefixed Firecracker processes under the
//! provider's socket dir, SIGKILLing them, and releasing their by-id host
//! resources (TAP / vsock / rootfs clone / DNAT, plus the template's sibling
//! rider TAP). It runs both as the first step of the warm-engine init and from
//! `reaper::reconcile_on_startup` (covering the warm-disabled-now case). A
//! `pkill -f 'fcwarm-'` alone is insufficient — it misses the `warm-*` pool
//! entries that actually hold the restored guest memory.

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
