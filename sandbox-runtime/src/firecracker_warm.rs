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
//! it never enters the sandbox store, so `SANDBOX_MAX_COUNT` and
//! `SANDBOX_HOST_MEMORY_BUDGET_MB` do not see it. Accounting applies at
//! claim time: the claim runs inside `firecracker::create_and_start`, which
//! the runtime layer only calls AFTER `admit_sandbox_resources` +
//! `enforce_sandbox_count_limit` have passed under the creation permit — a
//! warm claim is admitted exactly like a cold boot. Operators must size the
//! memory budget knowing the pool's own footprint sits outside it:
//! roughly `pool_size × 2 × mem_size_mib` with the `file` memory backend
//! (paused template + restored entry per generation), less with
//! `MICROVM_MEM_BACKEND=uffd` where entry pages fault in lazily.
//!
//! ## Fail-loud boundaries
//!
//! The single designed fallback is warm-miss → cold boot, and every miss
//! carries a typed [`WarmMiss`] reason that the create path logs. Seeding
//! failures are logged loudly and retried on subsequent creates
//! ([`WarmServing::ensure_seeding`]); a misconfigured
//! `SANDBOX_FC_WARM_POOL_SIZE` is a hard error, never a silent disable.
//!
//! ## Known limitations (documented, not silent)
//!
//! - Pool VMs are process-local: after an operator restart, templates and
//!   entries from the previous process are orphaned Firecracker processes
//!   the reaper cannot reconcile (they have no sandbox record). Host-level
//!   cleanup (`pkill -f 'fcwarm-'` or a reboot) is the remediation.
//! - A warm-claimed sandbox's alias resources (rider TAP, template vsock
//!   CID, template rootfs clone) are tracked in process memory; a restart
//!   leaks them until the host is reconciled the same way.

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

/// Snapshot name every generation's golden image is stored under.
pub(crate) const GOLDEN_SNAPSHOT_ID: &str = "golden";

/// Parse `SANDBOX_FC_WARM_POOL_SIZE`. Absent or `0` disables warm serving;
/// anything unparseable is a hard configuration error — a typo must never
/// silently disable the pool.
pub(crate) fn configured_pool_size() -> Result<usize> {
    match std::env::var("SANDBOX_FC_WARM_POOL_SIZE") {
        Err(_) => Ok(0),
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            trimmed.parse::<usize>().map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_FC_WARM_POOL_SIZE must be a non-negative integer, got {trimmed:?}"
                ))
            })
        }
    }
}

/// Parse `SANDBOX_FC_WARM_MAX_AGE_SECS` (pool-entry age eviction). Default
/// 3600s: evictions force a snapshot re-restore, so a long default keeps the
/// pool quiet; operators lower it if they want fresher entries.
pub(crate) fn configured_entry_max_age() -> Result<Duration> {
    match std::env::var("SANDBOX_FC_WARM_MAX_AGE_SECS") {
        Err(_) => Ok(Duration::from_secs(3600)),
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| {
                SandboxError::Validation(format!(
                    "SANDBOX_FC_WARM_MAX_AGE_SECS must be a non-negative integer, got {raw:?}"
                ))
            }),
    }
}

/// Parse `SANDBOX_FC_WARM_DISK_GB` — the per-generation rootfs clone size.
/// `0` (default) keeps the provider's workspace-default rootfs (no clone),
/// matching the cold path's `disk_gb == 0` semantics.
pub(crate) fn configured_warm_disk_gb() -> Result<u64> {
    match std::env::var("SANDBOX_FC_WARM_DISK_GB") {
        Err(_) => Ok(0),
        Ok(raw) => raw.trim().parse::<u64>().map_err(|_| {
            SandboxError::Validation(format!(
                "SANDBOX_FC_WARM_DISK_GB must be a non-negative integer, got {raw:?}"
            ))
        }),
    }
}

/// Shape a warm generation is provisioned at and a create request must match
/// (or leave unset) to be servable from the pool.
#[derive(Debug, Clone)]
pub(crate) struct WarmSettings {
    /// Number of generations to keep claimable.
    pub pool_size: usize,
    /// Stack the template's rootfs is cloned from when `disk_gb > 0`
    /// (`SANDBOX_FIRECRACKER_DEFAULT_STACK`). Irrelevant when `disk_gb == 0`
    /// — the cold path ignores `image` in that case too, so warm/cold serve
    /// identical guests.
    pub stack: Option<String>,
    /// Rootfs clone size for the template (`SANDBOX_FC_WARM_DISK_GB`).
    pub disk_gb: u64,
    /// vCPU count baked into the golden snapshot (provider workspace default).
    pub vcpu_count: u8,
    /// Memory baked into the golden snapshot (provider workspace default).
    pub mem_size_mib: u32,
    /// Pool-entry age eviction (`SANDBOX_FC_WARM_MAX_AGE_SECS`).
    pub entry_max_age: Duration,
}

/// Host-side composition the engine cannot do itself (TAP / vsock / rootfs /
/// guest readiness). Production wires the operator's `NetworkManager` /
/// `VsockManager` / `RootfsRegistry` (see `firecracker::OperatorWarmHost`);
/// tests and the network-less e2e substitute fakes.
#[async_trait]
pub(crate) trait WarmHost: Send + Sync + 'static {
    /// Attach the template's own network + vsock + (optional) rootfs clone,
    /// mutating `spec` in place. Returns the identity the generation's
    /// eventual claimant inherits.
    async fn compose_template(
        &self,
        template_id: &str,
        disk_gb: u64,
        stack: Option<&str>,
        spec: &mut VmSpec,
    ) -> Result<TemplateIdentity>;

    /// Block until the guest inside the freshly booted template is ready to
    /// be snapshotted (production: guest metadata daemon answers a ping).
    async fn await_guest_ready(&self, template_id: &str, identity: &TemplateIdentity)
    -> Result<()>;

    /// Called once after the golden snapshot is written, before the bucket
    /// is registered (i.e. before any restore can run). Production unlinks
    /// the template's vsock UDS *file*: the paused template keeps an
    /// orphaned listener fd (harmless — it never serves again), freeing the
    /// path so the restored entry's Firecracker can bind the vsock device
    /// recorded in the vmstate at the same path.
    async fn prepare_snapshot_source(&self, template_id: &str, identity: &TemplateIdentity);

    /// Attach the TAP the pooled entry restores onto. `None` when the host
    /// composes no network (network-less e2e) — the snapshot then carries no
    /// interface to override.
    async fn attach_rider(&self, rider_id: &str) -> Result<Option<NetworkInterface>>;

    /// Release template-keyed host resources. Flags select which: on claim,
    /// only the TAP is released (vsock CID + rootfs clone move to the
    /// claimed sandbox and are released at its delete); on seed failure,
    /// everything is. Rider TAPs are never released through this trait:
    /// `attach_rider` is the last fallible seeding step, and a claimed
    /// rider is released at sandbox delete (`firecracker::release_attachments`).
    async fn release_template(&self, template_id: &str, identity: &TemplateIdentity, all: bool);
}

/// Identity captured at template compose time, inherited by the claimant.
#[derive(Debug, Clone, Default)]
pub(crate) struct TemplateIdentity {
    /// Guest IP baked into the snapshot (kernel/network state). `None` only
    /// for hosts that compose no network.
    pub guest_ip: Option<Ipv4Addr>,
    /// Host UDS path of the template's vsock — the claimed VM's metadata
    /// listener binds here (recorded in the vmstate).
    pub metadata_uds: Option<PathBuf>,
    /// Whether a per-template rootfs clone was created (must be released
    /// when the claimed sandbox is deleted).
    pub rootfs_cloned: bool,
}

/// One claimable generation.
#[derive(Debug, Clone)]
pub(crate) struct Generation {
    pub template_id: String,
    pub rider_id: String,
    pub stack_key: StackKey,
    pub identity: TemplateIdentity,
    pub rider_attached: bool,
}

/// Everything the create path needs to finish provisioning a claimed VM.
#[derive(Debug, Clone)]
pub(crate) struct WarmClaim {
    /// Guest IP for the endpoint URL + DNAT (template's, via the snapshot).
    pub guest_ip: Option<Ipv4Addr>,
    /// UDS to inject per-sandbox env + sidecar token through.
    pub metadata_uds: Option<PathBuf>,
    /// Alias ids whose host resources now belong to the claimed sandbox.
    pub lineage: WarmLineage,
}

/// Host resources a warm-claimed sandbox holds under ids other than its own.
/// Released at sandbox delete in `firecracker::release_attachments`.
#[derive(Debug, Clone)]
pub(crate) struct WarmLineage {
    /// Template id: owns the vsock CID allocation and (when
    /// `rootfs_cloned`) the rootfs clone slot backing the claimed VM.
    pub template_id: String,
    /// Rider id: owns the TAP the claimed VM rides. `None` on network-less
    /// hosts.
    pub rider_id: Option<String>,
    pub rootfs_cloned: bool,
    /// Guest IP for endpoint rebuilds on resume (`firecracker::start`).
    pub guest_ip: Option<Ipv4Addr>,
}

/// Typed warm-miss reason. Logged by the create path before the designed
/// fallback to cold boot — the only silent thing about a miss is nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WarmMiss {
    /// `SANDBOX_FC_WARM_POOL_SIZE` is 0/unset.
    Disabled,
    /// Pool enabled but no generation is ready yet (seeding in flight or
    /// all seeds failed so far).
    NotReady,
    /// Ready generations exist but their buckets are empty (entry evicted
    /// and not yet refilled, or refill failing).
    Empty,
    DiskMismatch {
        requested: u64,
        pooled: u64,
    },
    ImageMismatch {
        requested: String,
        pooled: String,
    },
    CpuMismatch {
        requested: u64,
        pooled: u8,
    },
    MemoryMismatch {
        requested: u64,
        pooled: u32,
    },
    /// Handoff rename failed; the entry was destroyed and the generation
    /// returned to the pool for refill.
    RenameFailed(String),
    /// Post-rename resume failed; the claimed VM was destroyed and the
    /// generation returned to the pool for refill.
    ResumeFailed(String),
}

impl std::fmt::Display for WarmMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarmMiss::Disabled => write!(f, "warm pool disabled (SANDBOX_FC_WARM_POOL_SIZE=0)"),
            WarmMiss::NotReady => write!(f, "no warm generation ready (seeding in flight)"),
            WarmMiss::Empty => write!(f, "warm buckets empty (entry evicted, refill pending)"),
            WarmMiss::DiskMismatch { requested, pooled } => write!(
                f,
                "disk_gb mismatch (requested {requested}, pooled {pooled})"
            ),
            WarmMiss::ImageMismatch { requested, pooled } => {
                write!(
                    f,
                    "image mismatch (requested {requested:?}, pooled {pooled:?})"
                )
            }
            WarmMiss::CpuMismatch { requested, pooled } => {
                write!(
                    f,
                    "cpu_cores mismatch (requested {requested}, pooled {pooled})"
                )
            }
            WarmMiss::MemoryMismatch { requested, pooled } => {
                write!(
                    f,
                    "memory_mb mismatch (requested {requested}, pooled {pooled})"
                )
            }
            WarmMiss::RenameFailed(e) => write!(f, "warm handoff rename failed: {e}"),
            WarmMiss::ResumeFailed(e) => write!(f, "warm handoff resume failed: {e}"),
        }
    }
}

/// Outcome of a warm-claim attempt.
#[derive(Debug)]
pub(crate) enum WarmOutcome {
    Claimed(WarmClaim),
    Miss(WarmMiss),
}

/// Request shape the claim gate evaluates (subset of
/// `firecracker::FirecrackerCreateRequest` the engine needs — it must not
/// see env/labels, which are injected after the claim).
#[derive(Debug, Clone)]
pub(crate) struct WarmClaimRequest {
    pub sandbox_id: String,
    pub image: String,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
}

/// Validates pool entries by provider record: present and paused
/// (`Stopped`). Catches destroyed/renamed-away records; it cannot see a
/// crashed FC child behind a live record — the claim path's resume would
/// surface that as `ResumeFailed` (logged miss, cold fallback).
struct PausedEntryValidator<P: VmQuery> {
    provider: P,
}

impl<P: VmQuery + Send + Sync + 'static> EntryValidator for PausedEntryValidator<P> {
    fn validate(&self, vm_id: &str) -> ValidationResult {
        match self.provider.get_vm(vm_id) {
            Ok(Some(view)) if view.status == VmStatus::Stopped => ValidationResult::Healthy,
            Ok(Some(view)) => {
                ValidationResult::Unhealthy(format!("entry {vm_id} in state {}", view.status))
            }
            Ok(None) => ValidationResult::Unhealthy(format!("entry {vm_id} has no record")),
            Err(e) => ValidationResult::Unhealthy(format!("entry {vm_id} query failed: {e}")),
        }
    }
}

/// Seed/claim counters, exposed alongside [`WarmPoolMetrics`].
#[derive(Debug, Default)]
struct WarmCounters {
    claims: AtomicU64,
    misses: AtomicU64,
    seed_failures: AtomicU64,
}

/// The warm-serving engine. Generic over the provider so tests drive it with
/// an in-memory fake and the e2e with the real `FirecrackerVmProvider`.
pub(crate) struct WarmServing<P: VmProvider + VmQuery + Clone + 'static> {
    provider: P,
    host: Arc<dyn WarmHost>,
    settings: WarmSettings,
    pool: WarmPool<P>,
    ready: Mutex<Vec<Generation>>,
    seeds_in_flight: Arc<AtomicUsize>,
    generation_counter: AtomicU64,
    counters: WarmCounters,
}

impl<P: VmProvider + VmQuery + Clone + 'static> WarmServing<P> {
    pub(crate) fn new(provider: P, host: Arc<dyn WarmHost>, settings: WarmSettings) -> Self {
        let pool = WarmPool::start(
            provider.clone(),
            WarmPoolConfig {
                // Depth 1 per generation: a generation's identity (rider
                // TAP, vsock UDS, rootfs file) is single-occupancy, so a
                // deeper bucket would restore entries that collide on it.
                min_depth: 1,
                max_depth: 1,
                refill_interval: Duration::from_secs(2),
                entry_max_age: settings.entry_max_age,
            },
            Arc::new(PausedEntryValidator {
                provider: provider.clone(),
            }),
        );
        Self {
            provider,
            host,
            settings,
            pool,
            ready: Mutex::new(Vec::new()),
            seeds_in_flight: Arc::new(AtomicUsize::new(0)),
            generation_counter: AtomicU64::new(0),
            counters: WarmCounters::default(),
        }
    }

    /// Number of generations currently claimable (bucket may still be
    /// refilling; see [`WarmMiss::Empty`]).
    pub(crate) fn ready_generations(&self) -> usize {
        lock_ready(&self.ready).len()
    }

    #[cfg(test)]
    pub(crate) fn pool_metrics(&self) -> WarmPoolMetrics {
        self.pool.metrics()
    }

    /// Top the pool back up to `pool_size` generations. Called by the create
    /// path on every request: cheap when saturated, and it doubles as the
    /// retry loop for failed seeds (no supervisor thread to babysit).
    pub(crate) fn ensure_seeding(self: &Arc<Self>) {
        loop {
            let ready = self.ready_generations();
            let in_flight = self.seeds_in_flight.load(Ordering::SeqCst);
            if ready + in_flight >= self.settings.pool_size {
                return;
            }
            // Reserve the slot before spawning so concurrent creates cannot
            // over-seed past pool_size.
            if self
                .seeds_in_flight
                .compare_exchange(in_flight, in_flight + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                continue;
            }
            let this = Arc::clone(self);
            tokio::spawn(async move {
                let outcome = this.seed_generation().await;
                this.seeds_in_flight.fetch_sub(1, Ordering::SeqCst);
                if let Err(err) = outcome {
                    this.counters.seed_failures.fetch_add(1, Ordering::Relaxed);
                    tracing::error!(%err, "firecracker warm-pool generation seed failed");
                }
            });
        }
    }

    /// Cold-boot a template, snapshot it, and register the generation's
    /// bucket. The pool's refill thread pre-restores the entry afterwards.
    pub(crate) async fn seed_generation(&self) -> Result<()> {
        let generation = self.generation_counter.fetch_add(1, Ordering::SeqCst);
        let template_id = format!("fcwarm-g{generation}-tpl");
        let rider_id = format!("fcwarm-g{generation}-rider");

        let mut spec = VmSpec {
            vcpu_count: Some(self.settings.vcpu_count),
            mem_size_mib: Some(self.settings.mem_size_mib),
            ..VmSpec::default()
        };
        let identity = self
            .host
            .compose_template(
                &template_id,
                self.settings.disk_gb,
                self.settings.stack.as_deref(),
                &mut spec,
            )
            .await?;

        if let Err(err) = self
            .boot_pause_snapshot(&template_id, &spec, &identity)
            .await
        {
            // Roll back everything the template holds; the VM itself is
            // destroyed (best-effort) inside boot_pause_snapshot.
            self.host
                .release_template(&template_id, &identity, true)
                .await;
            return Err(err);
        }

        self.host
            .prepare_snapshot_source(&template_id, &identity)
            .await;

        let rider_iface = match self.host.attach_rider(&rider_id).await {
            Ok(iface) => iface,
            Err(err) => {
                self.destroy_vm_best_effort(&template_id).await;
                self.host
                    .release_template(&template_id, &identity, true)
                    .await;
                return Err(err);
            }
        };
        let rider_attached = rider_iface.is_some();

        let stack_key = StackKey {
            stack_name: self
                .settings
                .stack
                .clone()
                .unwrap_or_else(|| "workspace-default".to_string()),
            version: format!("g{generation}"),
            vcpu_count: self.settings.vcpu_count,
            mem_size_mib: self.settings.mem_size_mib,
        };
        self.pool.register(
            stack_key.clone(),
            SnapshotRef {
                vm_id: template_id.clone(),
                snapshot_id: GOLDEN_SNAPSHOT_ID.to_string(),
                // Entries wait paused; the claim resumes after rename so a
                // pooled guest burns no CPU and its clock is corrected at
                // handoff time, not pool-idle time.
                resume_immediately: false,
                network_overrides: rider_iface.into_iter().collect(),
            },
        );

        lock_ready(&self.ready).push(Generation {
            template_id: template_id.clone(),
            rider_id,
            stack_key,
            identity,
            rider_attached,
        });
        tracing::info!(
            template_id,
            generation,
            "firecracker warm-pool generation seeded (golden snapshot registered)"
        );
        Ok(())
    }

    async fn boot_pause_snapshot(
        &self,
        template_id: &str,
        spec: &VmSpec,
        identity: &TemplateIdentity,
    ) -> Result<()> {
        let create_id = template_id.to_string();
        let create_spec = spec.clone();
        let provider = self.provider.clone();
        run_provider(
            move || provider.create_vm_with_spec(&create_id, &create_spec),
            "warm template create",
            template_id,
        )
        .await?;

        let result: Result<()> = async {
            let start_id = template_id.to_string();
            let provider = self.provider.clone();
            run_provider(
                move || provider.start_vm(&start_id),
                "warm template start",
                template_id,
            )
            .await?;

            self.host.await_guest_ready(template_id, identity).await?;

            let pause_id = template_id.to_string();
            let provider = self.provider.clone();
            run_provider(
                move || provider.stop_vm(&pause_id),
                "warm template pause",
                template_id,
            )
            .await?;

            let snap_id = template_id.to_string();
            let provider = self.provider.clone();
            run_provider(
                move || provider.snapshot_vm(&snap_id, GOLDEN_SNAPSHOT_ID),
                "warm template snapshot",
                template_id,
            )
            .await?;
            Ok(())
        }
        .await;

        if result.is_err() {
            self.destroy_vm_best_effort(template_id).await;
        }
        result
    }

    /// Try to serve `request` from the pool. Never falls back internally —
    /// the caller owns the (logged) cold-boot fallback.
    pub(crate) async fn claim(&self, request: &WarmClaimRequest) -> WarmOutcome {
        if let Some(miss) = self.shape_gate(request) {
            return self.miss(miss);
        }

        // Pop a claimable generation whose bucket has an entry. The
        // generation leaves `ready` before any await so a concurrent claim
        // cannot double-serve it; on failure it is pushed back (its
        // template + snapshot still exist, so the refill thread restores a
        // replacement entry).
        let (generation, handle) = {
            let mut ready = lock_ready(&self.ready);
            let mut found = None;
            for (idx, generation) in ready.iter().enumerate() {
                if let Some(handle) = self.pool.acquire(&generation.stack_key) {
                    found = Some((idx, handle));
                    break;
                }
            }
            match found {
                Some((idx, handle)) => (ready.remove(idx), handle),
                None => {
                    drop(ready);
                    let reason = if self.ready_generations() == 0 {
                        WarmMiss::NotReady
                    } else {
                        WarmMiss::Empty
                    };
                    return self.miss(reason);
                }
            }
        };

        let entry_id = handle.source_vm_id.clone();
        let sandbox_id = request.sandbox_id.clone();

        // Handoff: re-key the pre-restored entry onto the sandbox id.
        let rename_provider = self.provider.clone();
        let rename_entry = entry_id.clone();
        let rename_target = sandbox_id.clone();
        if let Err(err) = run_provider(
            move || rename_provider.rename_vm(&rename_entry, &rename_target),
            "warm handoff rename",
            &entry_id,
        )
        .await
        {
            self.destroy_vm_best_effort(&entry_id).await;
            lock_ready(&self.ready).push(generation);
            return self.miss(WarmMiss::RenameFailed(err.to_string()));
        }

        // Entries are restored paused; resume under the new identity.
        let resume_provider = self.provider.clone();
        let resume_id = sandbox_id.clone();
        if let Err(err) = run_provider(
            move || resume_provider.start_vm(&resume_id),
            "warm handoff resume",
            &sandbox_id,
        )
        .await
        {
            self.destroy_vm_best_effort(&sandbox_id).await;
            lock_ready(&self.ready).push(generation);
            return self.miss(WarmMiss::ResumeFailed(err.to_string()));
        }

        // The generation is spent: its identity now belongs to the claimed
        // sandbox. Unregister the bucket (a refill racing this window would
        // restore an entry the unregister immediately destroys — upstream
        // WarmPool destroys orphans inserted into unregistered buckets),
        // destroy the paused template (frees its guest memory and TAP; the
        // snapshot artifacts die with it, which is fine — nothing restores
        // from a spent generation), and keep the vsock CID + rootfs clone
        // alive under the template id until the sandbox is deleted.
        self.pool.unregister(&generation.stack_key);
        self.destroy_vm_best_effort(&generation.template_id).await;
        self.host
            .release_template(&generation.template_id, &generation.identity, false)
            .await;

        self.counters.claims.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            sandbox_id = %request.sandbox_id,
            template_id = %generation.template_id,
            "firecracker warm-pool claim served (snapshot-restored VM handed off via rename)"
        );

        WarmOutcome::Claimed(WarmClaim {
            guest_ip: generation.identity.guest_ip,
            metadata_uds: generation.identity.metadata_uds.clone(),
            lineage: WarmLineage {
                template_id: generation.template_id,
                rider_id: generation.rider_attached.then_some(generation.rider_id),
                rootfs_cloned: generation.identity.rootfs_cloned,
                guest_ip: generation.identity.guest_ip,
            },
        })
    }

    /// `None` = request matches the pooled shape.
    fn shape_gate(&self, request: &WarmClaimRequest) -> Option<WarmMiss> {
        if request.disk_gb != self.settings.disk_gb {
            return Some(WarmMiss::DiskMismatch {
                requested: request.disk_gb,
                pooled: self.settings.disk_gb,
            });
        }
        // When disk_gb == 0 the cold path boots the workspace-default rootfs
        // regardless of `image`, so warm serves any image for parity. With a
        // cloned template the image must name the pooled stack (or be empty
        // and default to it).
        if self.settings.disk_gb > 0 {
            let pooled = self.settings.stack.clone().unwrap_or_default();
            let requested = request.image.trim();
            if !requested.is_empty() && requested != pooled {
                return Some(WarmMiss::ImageMismatch {
                    requested: requested.to_string(),
                    pooled,
                });
            }
        }
        if request.cpu_cores != 0 && request.cpu_cores != u64::from(self.settings.vcpu_count) {
            return Some(WarmMiss::CpuMismatch {
                requested: request.cpu_cores,
                pooled: self.settings.vcpu_count,
            });
        }
        if request.memory_mb != 0 && request.memory_mb != u64::from(self.settings.mem_size_mib) {
            return Some(WarmMiss::MemoryMismatch {
                requested: request.memory_mb,
                pooled: self.settings.mem_size_mib,
            });
        }
        None
    }

    fn miss(&self, miss: WarmMiss) -> WarmOutcome {
        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        WarmOutcome::Miss(miss)
    }

    async fn destroy_vm_best_effort(&self, vm_id: &str) {
        let provider = self.provider.clone();
        let id = vm_id.to_string();
        let owner = vm_id.to_string();
        if let Err(err) =
            run_provider(move || provider.destroy_vm(&id), "warm destroy", &owner).await
        {
            tracing::warn!(vm_id = %owner, %err, "failed destroying warm-pool VM");
        }
    }
}

/// Provider calls shell out / do unix-socket I/O; keep them off the async
/// runtime. Mirrors `firecracker.rs`'s spawn_blocking convention.
async fn run_provider<F>(f: F, action: &str, vm_id: &str) -> Result<()>
where
    F: FnOnce() -> std::result::Result<(), microvm_runtime::error::VmRuntimeError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| SandboxError::Unavailable(format!("{action} join error for {vm_id}: {e}")))?
        .map_err(|e| SandboxError::Unavailable(format!("{action} for {vm_id}: {e}")))
}

fn lock_ready(m: &Mutex<Vec<Generation>>) -> std::sync::MutexGuard<'_, Vec<Generation>> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Test / e2e host that composes no network, no vsock, and no rootfs clone.
/// The engine's lifecycle (seed → refill → claim → retire) is identical;
/// only the identity fields are empty. Also used by the KVM-gated e2e where
/// TAP creation would require root.
#[cfg(test)]
pub(crate) struct NoNetworkWarmHost {
    /// Simulated guest-ready delay (the e2e uses a real boot-settle wait).
    pub ready_delay: Duration,
}

#[cfg(test)]
#[async_trait]
impl WarmHost for NoNetworkWarmHost {
    async fn compose_template(
        &self,
        _template_id: &str,
        _disk_gb: u64,
        _stack: Option<&str>,
        _spec: &mut VmSpec,
    ) -> Result<TemplateIdentity> {
        Ok(TemplateIdentity::default())
    }

    async fn await_guest_ready(
        &self,
        _template_id: &str,
        _identity: &TemplateIdentity,
    ) -> Result<()> {
        if !self.ready_delay.is_zero() {
            tokio::time::sleep(self.ready_delay).await;
        }
        Ok(())
    }

    async fn prepare_snapshot_source(&self, _template_id: &str, _identity: &TemplateIdentity) {}

    async fn attach_rider(&self, _rider_id: &str) -> Result<Option<NetworkInterface>> {
        Ok(None)
    }

    async fn release_template(&self, _template_id: &str, _identity: &TemplateIdentity, _all: bool) {
    }
}

#[cfg(test)]
pub(crate) fn test_settings(pool_size: usize) -> WarmSettings {
    WarmSettings {
        pool_size,
        stack: None,
        disk_gb: 0,
        vcpu_count: 2,
        mem_size_mib: 1024,
        entry_max_age: Duration::from_secs(3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;
    use std::time::Instant;

    /// In-memory provider with full lifecycle + rename + failure injection.
    /// `microvm_runtime::InMemoryVmProvider` lacks `rename_vm` (falls to the
    /// trait's `Unsupported` default) and ignores specs, so the claim path
    /// needs this richer fake.
    #[derive(Clone, Default)]
    struct TestProvider {
        state: Arc<Mutex<StdHashMap<String, TestVm>>>,
        fail_rename: Arc<Mutex<Option<String>>>,
        fail_start_of: Arc<Mutex<Option<String>>>,
    }

    #[derive(Debug, Clone)]
    struct TestVm {
        status: VmStatus,
        snapshots: Vec<String>,
        restored_from: Option<SnapshotRef>,
    }

    impl TestProvider {
        fn vm(&self, id: &str) -> Option<TestVm> {
            self.state.lock().unwrap().get(id).cloned()
        }

        fn live_count(&self) -> usize {
            self.state
                .lock()
                .unwrap()
                .values()
                .filter(|vm| vm.status != VmStatus::Destroyed)
                .count()
        }
    }

    impl VmProvider for TestProvider {
        fn create_vm(&self, vm_id: &str) -> microvm_runtime::error::VmRuntimeResult<()> {
            self.create_vm_with_spec(vm_id, &VmSpec::default())
        }

        fn create_vm_with_spec(
            &self,
            vm_id: &str,
            spec: &VmSpec,
        ) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            let mut state = self.state.lock().unwrap();
            if state.contains_key(vm_id) {
                return Err(VmRuntimeError::VmAlreadyExists(vm_id.into()));
            }
            // Mirror the real adapter: restoring from a missing snapshot is
            // a typed failure, and a restored VM lands paused when
            // `resume_immediately` is false.
            let (status, restored_from) = match spec.restore_from.clone() {
                Some(snap) => {
                    let source_has_snapshot = state
                        .get(&snap.vm_id)
                        .map(|vm| {
                            vm.status != VmStatus::Destroyed
                                && vm.snapshots.iter().any(|s| s == &snap.snapshot_id)
                        })
                        .unwrap_or(false);
                    if !source_has_snapshot {
                        return Err(VmRuntimeError::SnapshotNotFound {
                            vm_id: snap.vm_id.clone(),
                            snapshot_id: snap.snapshot_id.clone(),
                        });
                    }
                    let status = if snap.resume_immediately {
                        VmStatus::Running
                    } else {
                        VmStatus::Stopped
                    };
                    (status, Some(snap))
                }
                None => (VmStatus::Created, None),
            };
            state.insert(
                vm_id.into(),
                TestVm {
                    status,
                    snapshots: Vec::new(),
                    restored_from,
                },
            );
            Ok(())
        }

        fn start_vm(&self, vm_id: &str) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            if self.fail_start_of.lock().unwrap().as_deref() == Some(vm_id) {
                return Err(VmRuntimeError::Shutdown("scripted start failure".into()));
            }
            let mut state = self.state.lock().unwrap();
            let vm = state
                .get_mut(vm_id)
                .ok_or_else(|| VmRuntimeError::VmNotFound(vm_id.into()))?;
            match vm.status {
                VmStatus::Created | VmStatus::Stopped => {
                    vm.status = VmStatus::Running;
                    Ok(())
                }
                other => Err(VmRuntimeError::InvalidTransition {
                    vm_id: vm_id.into(),
                    from: other.to_string(),
                    to: "running",
                }),
            }
        }

        fn stop_vm(&self, vm_id: &str) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            let mut state = self.state.lock().unwrap();
            let vm = state
                .get_mut(vm_id)
                .ok_or_else(|| VmRuntimeError::VmNotFound(vm_id.into()))?;
            match vm.status {
                VmStatus::Running => {
                    vm.status = VmStatus::Stopped;
                    Ok(())
                }
                other => Err(VmRuntimeError::InvalidTransition {
                    vm_id: vm_id.into(),
                    from: other.to_string(),
                    to: "stopped",
                }),
            }
        }

        fn snapshot_vm(
            &self,
            vm_id: &str,
            snapshot_id: &str,
        ) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            let mut state = self.state.lock().unwrap();
            let vm = state
                .get_mut(vm_id)
                .ok_or_else(|| VmRuntimeError::VmNotFound(vm_id.into()))?;
            if vm.status == VmStatus::Destroyed {
                return Err(VmRuntimeError::InvalidTransition {
                    vm_id: vm_id.into(),
                    from: vm.status.to_string(),
                    to: "snapshot",
                });
            }
            vm.snapshots.push(snapshot_id.into());
            Ok(())
        }

        fn destroy_vm(&self, vm_id: &str) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            let mut state = self.state.lock().unwrap();
            let vm = state
                .get_mut(vm_id)
                .ok_or_else(|| VmRuntimeError::VmNotFound(vm_id.into()))?;
            vm.status = VmStatus::Destroyed;
            Ok(())
        }

        fn rename_vm(
            &self,
            old_vm_id: &str,
            new_vm_id: &str,
        ) -> microvm_runtime::error::VmRuntimeResult<()> {
            use microvm_runtime::error::VmRuntimeError;
            if self.fail_rename.lock().unwrap().as_deref() == Some(old_vm_id) {
                return Err(VmRuntimeError::Unsupported(
                    "scripted rename failure".into(),
                ));
            }
            let mut state = self.state.lock().unwrap();
            if state.contains_key(new_vm_id) {
                return Err(VmRuntimeError::VmAlreadyExists(new_vm_id.into()));
            }
            let vm = state
                .remove(old_vm_id)
                .ok_or_else(|| VmRuntimeError::VmNotFound(old_vm_id.into()))?;
            state.insert(new_vm_id.into(), vm);
            Ok(())
        }
    }

    impl VmQuery for TestProvider {
        fn list_vms(
            &self,
        ) -> microvm_runtime::error::VmRuntimeResult<Vec<microvm_runtime::model::VmView>> {
            let state = self.state.lock().unwrap();
            Ok(state
                .iter()
                .map(|(id, vm)| microvm_runtime::model::VmView {
                    vm_id: id.clone(),
                    status: vm.status,
                    snapshots: vm.snapshots.clone(),
                })
                .collect())
        }

        fn get_vm(
            &self,
            vm_id: &str,
        ) -> microvm_runtime::error::VmRuntimeResult<Option<microvm_runtime::model::VmView>>
        {
            let state = self.state.lock().unwrap();
            Ok(state.get(vm_id).map(|vm| microvm_runtime::model::VmView {
                vm_id: vm_id.into(),
                status: vm.status,
                snapshots: vm.snapshots.clone(),
            }))
        }

        fn list_snapshots(
            &self,
            vm_id: &str,
        ) -> microvm_runtime::error::VmRuntimeResult<Option<Vec<String>>> {
            let state = self.state.lock().unwrap();
            Ok(state.get(vm_id).map(|vm| vm.snapshots.clone()))
        }
    }

    fn no_network_host() -> Arc<dyn WarmHost> {
        Arc::new(NoNetworkWarmHost {
            ready_delay: Duration::ZERO,
        })
    }

    fn request(sandbox_id: &str) -> WarmClaimRequest {
        WarmClaimRequest {
            sandbox_id: sandbox_id.into(),
            image: String::new(),
            cpu_cores: 0,
            memory_mb: 0,
            disk_gb: 0,
        }
    }

    async fn wait_for_entry<P: VmProvider + VmQuery + Clone + 'static>(serving: &WarmServing<P>) {
        // The pool's own refill thread restores the entry asynchronously.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if serving.pool_metrics().created_total >= 1 {
                // created + validated: give the insert a beat to complete.
                tokio::time::sleep(Duration::from_millis(50)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("warm pool never restored an entry");
    }

    /// Named bug: claim path skips the rename handoff (e.g. hands back the
    /// entry under its pool id), so the sandbox id never owns the VM.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn seed_then_claim_hands_off_via_rename_and_retires_generation() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            no_network_host(),
            test_settings(1),
        ));

        serving.seed_generation().await.expect("seed");
        assert_eq!(serving.ready_generations(), 1);
        let template = provider.vm("fcwarm-g0-tpl").expect("template exists");
        assert_eq!(
            template.status,
            VmStatus::Stopped,
            "template paused post-snapshot"
        );
        assert_eq!(template.snapshots, vec![GOLDEN_SNAPSHOT_ID.to_string()]);

        wait_for_entry(&serving).await;

        let outcome = serving.claim(&request("sandbox-1")).await;
        let claim = match outcome {
            WarmOutcome::Claimed(c) => c,
            WarmOutcome::Miss(m) => panic!("expected claim, got miss: {m}"),
        };
        assert_eq!(claim.lineage.template_id, "fcwarm-g0-tpl");

        // The sandbox id owns a RUNNING VM restored from the golden
        // snapshot; the entry's pool id no longer exists (renamed away).
        let claimed = provider.vm("sandbox-1").expect("claimed VM exists");
        assert_eq!(claimed.status, VmStatus::Running);
        let restored = claimed
            .restored_from
            .expect("claimed VM was snapshot-restored");
        assert_eq!(restored.vm_id, "fcwarm-g0-tpl");
        assert_eq!(restored.snapshot_id, GOLDEN_SNAPSHOT_ID);

        // Generation retired: template destroyed, bucket unregistered,
        // nothing claimable until the next ensure_seeding pass.
        assert_eq!(
            provider.vm("fcwarm-g0-tpl").expect("record kept").status,
            VmStatus::Destroyed
        );
        assert_eq!(serving.ready_generations(), 0);
        match serving.claim(&request("sandbox-2")).await {
            WarmOutcome::Miss(WarmMiss::NotReady) => {}
            other => panic!("spent generation must not serve again: {other:?}"),
        }
    }

    /// Named bug: the shape gate silently serves a mismatched request, so a
    /// tenant asking for 4 GiB gets a 1 GiB pooled VM.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shape_gate_rejects_mismatches_with_typed_reasons() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider,
            no_network_host(),
            test_settings(1),
        ));
        serving.seed_generation().await.expect("seed");
        wait_for_entry(&serving).await;

        let mut req = request("sb-mem");
        req.memory_mb = 4096;
        assert!(matches!(
            serving.claim(&req).await,
            WarmOutcome::Miss(WarmMiss::MemoryMismatch {
                requested: 4096,
                pooled: 1024
            })
        ));

        let mut req = request("sb-cpu");
        req.cpu_cores = 8;
        assert!(matches!(
            serving.claim(&req).await,
            WarmOutcome::Miss(WarmMiss::CpuMismatch {
                requested: 8,
                pooled: 2
            })
        ));

        let mut req = request("sb-disk");
        req.disk_gb = 20;
        assert!(matches!(
            serving.claim(&req).await,
            WarmOutcome::Miss(WarmMiss::DiskMismatch {
                requested: 20,
                pooled: 0
            })
        ));

        // disk_gb == 0 → image is irrelevant (cold path boots the workspace
        // default rootfs for any image) — must still claim.
        let mut req = request("sb-img");
        req.image = "node-20".into();
        assert!(matches!(serving.claim(&req).await, WarmOutcome::Claimed(_)));
    }

    /// Named bug: with a cloned template (disk_gb > 0) a request for a
    /// different stack image is served the pooled stack's guest.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cloned_template_gates_on_image() {
        let provider = TestProvider::default();
        let mut settings = test_settings(1);
        settings.disk_gb = 8;
        settings.stack = Some("node-20".into());
        let serving = Arc::new(WarmServing::new(provider, no_network_host(), settings));
        serving.seed_generation().await.expect("seed");
        wait_for_entry(&serving).await;

        let mut req = request("sb-wrong-img");
        req.disk_gb = 8;
        req.image = "python-312".into();
        assert!(matches!(
            serving.claim(&req).await,
            WarmOutcome::Miss(WarmMiss::ImageMismatch { .. })
        ));

        let mut req = request("sb-right-img");
        req.disk_gb = 8;
        req.image = "node-20".into();
        assert!(matches!(serving.claim(&req).await, WarmOutcome::Claimed(_)));
    }

    /// Named bug: a failed rename consumes the generation (pool goes dark
    /// after a transient handoff error instead of refilling and recovering).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_failure_returns_generation_for_refill_then_recovers() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            no_network_host(),
            test_settings(1),
        ));
        serving.seed_generation().await.expect("seed");
        wait_for_entry(&serving).await;

        // Script the first entry's rename to fail.
        let entry_id = provider
            .state
            .lock()
            .unwrap()
            .iter()
            .find(|(id, vm)| id.starts_with("warm-") && vm.status == VmStatus::Stopped)
            .map(|(id, _)| id.clone())
            .expect("pooled entry exists");
        *provider.fail_rename.lock().unwrap() = Some(entry_id.clone());

        match serving.claim(&request("sandbox-1")).await {
            WarmOutcome::Miss(WarmMiss::RenameFailed(_)) => {}
            other => panic!("expected RenameFailed, got {other:?}"),
        }
        // Failed entry destroyed, generation back in rotation.
        assert_eq!(
            provider.vm(&entry_id).expect("entry record").status,
            VmStatus::Destroyed
        );
        assert_eq!(serving.ready_generations(), 1);

        // Refill restores a replacement entry; the next claim succeeds.
        *provider.fail_rename.lock().unwrap() = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let WarmOutcome::Claimed(_) = serving.claim(&request("sandbox-2")).await {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "pool never recovered from rename failure"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Named bug: resume failure leaves a paused VM parked under the tenant
    /// sandbox id (a dead sandbox that looks provisioned).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_failure_destroys_claimed_vm_and_reports_miss() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            no_network_host(),
            test_settings(1),
        ));
        serving.seed_generation().await.expect("seed");
        wait_for_entry(&serving).await;

        *provider.fail_start_of.lock().unwrap() = Some("sandbox-1".into());
        match serving.claim(&request("sandbox-1")).await {
            WarmOutcome::Miss(WarmMiss::ResumeFailed(_)) => {}
            other => panic!("expected ResumeFailed, got {other:?}"),
        }
        assert_eq!(
            provider.vm("sandbox-1").expect("record").status,
            VmStatus::Destroyed,
            "half-claimed VM must not survive under the sandbox id"
        );
        assert_eq!(serving.ready_generations(), 1, "generation stays claimable");
    }

    /// Named bug: ensure_seeding over-seeds past pool_size when creates race.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ensure_seeding_caps_at_pool_size() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            no_network_host(),
            test_settings(2),
        ));
        for _ in 0..8 {
            serving.ensure_seeding();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while serving.ready_generations() < 2 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(serving.ready_generations(), 2);
        // Templates g0 + g1 only — racing ensure_seeding calls must not
        // have seeded extra generations.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let templates = provider
            .state
            .lock()
            .unwrap()
            .keys()
            .filter(|id| id.ends_with("-tpl"))
            .count();
        assert_eq!(templates, 2);
    }

    /// The admission invariant, engine side: pool inventory lives only in
    /// the provider — seeding N generations must not create or touch any
    /// sandbox-store record (the engine has no store access at all; this
    /// pins the VM-count shape so inventory stays distinguishable).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pool_inventory_is_template_plus_entry_only() {
        let provider = TestProvider::default();
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            no_network_host(),
            test_settings(1),
        ));
        serving.seed_generation().await.expect("seed");
        wait_for_entry(&serving).await;
        // Exactly two live VMs: the paused template + the paused entry.
        assert_eq!(provider.live_count(), 2);
        let state = provider.state.lock().unwrap();
        for id in state.keys() {
            assert!(
                id.starts_with("fcwarm-") || id.starts_with("warm-"),
                "pool inventory must be namespaced, found {id}"
            );
        }
    }

    #[test]
    fn pool_size_env_parses_and_fails_loud() {
        let _guard = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prior = std::env::var("SANDBOX_FC_WARM_POOL_SIZE").ok();

        unsafe { std::env::remove_var("SANDBOX_FC_WARM_POOL_SIZE") };
        assert_eq!(configured_pool_size().expect("absent = disabled"), 0);

        unsafe { std::env::set_var("SANDBOX_FC_WARM_POOL_SIZE", "3") };
        assert_eq!(configured_pool_size().expect("valid"), 3);

        // Named bug: a typo'd value silently disables the pool instead of
        // erroring — the operator believes warm serving is on.
        unsafe { std::env::set_var("SANDBOX_FC_WARM_POOL_SIZE", "two") };
        let err = configured_pool_size().expect_err("invalid must be a hard error");
        assert!(matches!(err, SandboxError::Validation(_)), "got {err:?}");

        match prior {
            Some(v) => unsafe { std::env::set_var("SANDBOX_FC_WARM_POOL_SIZE", v) },
            None => unsafe { std::env::remove_var("SANDBOX_FC_WARM_POOL_SIZE") },
        }
    }

    // ---- KVM-gated e2e (real Firecracker) --------------------------------
    //
    // Follows the primitive's convention for real-VMM tests (`#[ignore]` +
    // explicit invocation; the in-process contract tests in
    // `tests/firecracker_in_process.rs` point at that convention). Run:
    //
    // ```sh
    // FC_E2E_BIN=/usr/local/bin/firecracker-v1.12 \
    //   cargo test -p sandbox-runtime --features test-utils --lib -- \
    //   --ignored fc_warm_e2e --nocapture
    // ```
    //
    // Requires /dev/kvm (rw), a Firecracker binary, and kernel + rootfs
    // images (defaults below match the microvm-runtime workspace layout).
    // No root needed: the e2e composes no TAP / jailer.

    fn e2e_config(
        tmp: &std::path::Path,
    ) -> microvm_runtime::adapters::firecracker::FirecrackerConfig {
        use microvm_runtime::adapters::firecracker::MemBackend;
        let env_or =
            |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.to_string());
        let binary = env_or("FC_E2E_BIN", "/usr/local/bin/firecracker");
        let kernel = env_or("FC_E2E_KERNEL", "/var/lib/firecracker/vmlinux");
        let rootfs = env_or("FC_E2E_ROOTFS", "/var/lib/firecracker/rootfs/base.ext4");
        for (what, path) in [
            ("kvm", "/dev/kvm"),
            ("binary", &binary),
            ("kernel", &kernel),
            ("rootfs", &rootfs),
        ] {
            assert!(
                std::path::Path::new(path).exists(),
                "e2e prerequisite missing: {what} at {path} (override with FC_E2E_BIN/KERNEL/ROOTFS)"
            );
        }
        microvm_runtime::adapters::firecracker::FirecrackerConfig {
            binary_path: binary.into(),
            kernel_path: kernel.into(),
            rootfs_path: rootfs.into(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off".into(),
            socket_dir: tmp.join("sockets"),
            state_dir: tmp.join("state"),
            vcpu_count: 1,
            mem_size_mib: 256,
            rootfs_read_only: true,
            api_timeout: Duration::from_secs(5),
            socket_ready_timeout: Duration::from_secs(5),
            mem_backend: MemBackend::File,
        }
    }

    fn millis(d: Duration) -> f64 {
        d.as_secs_f64() * 1e3
    }

    fn median(mut xs: Vec<f64>) -> f64 {
        xs.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        xs[xs.len() / 2]
    }

    /// Cold create→running vs warm claim (acquire → rename → resume)
    /// against a real Firecracker, through the same engine production uses.
    /// Prints per-iteration wall clocks; asserts the claim median beats the
    /// cold median (the entire point of the pool).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires /dev/kvm, firecracker binary, kernel + rootfs images"]
    async fn fc_warm_e2e_cold_vs_claim() {
        use microvm_runtime::adapters::firecracker::FirecrackerVmProvider;

        let tmp = tempfile::tempdir().expect("tempdir");
        let provider = FirecrackerVmProvider::new(e2e_config(tmp.path()));
        let settings = WarmSettings {
            pool_size: 1,
            stack: None,
            disk_gb: 0,
            vcpu_count: 1,
            mem_size_mib: 256,
            entry_max_age: Duration::from_secs(3600),
        };
        let serving = Arc::new(WarmServing::new(
            provider.clone(),
            Arc::new(NoNetworkWarmHost {
                // Boot-settle before pausing for the golden snapshot,
                // mirroring the primitive's own e2e.
                ready_delay: Duration::from_secs(2),
            }),
            settings,
        ));

        const N: usize = 5;
        let mut cold_ms = Vec::with_capacity(N);
        let mut claim_ms = Vec::with_capacity(N);

        for i in 0..N {
            // Cold boot: FC process spawn + configure + InstanceStart.
            let cold_id = format!("e2e-cold-{i}");
            let spec = VmSpec {
                vcpu_count: Some(1),
                mem_size_mib: Some(256),
                ..VmSpec::default()
            };
            let started = Instant::now();
            provider
                .create_vm_with_spec(&cold_id, &spec)
                .expect("cold create");
            provider.start_vm(&cold_id).expect("cold start");
            cold_ms.push(millis(started.elapsed()));
            provider.destroy_vm(&cold_id).expect("cold destroy");

            // Warm claim: seed a generation (cold boot + snapshot, prepaid),
            // wait for the pool's refill thread to pre-restore the entry,
            // then measure only what a tenant create would pay.
            serving.seed_generation().await.expect("seed");
            wait_for_entry_nth(&serving, i as u64 + 1).await;
            let sandbox_id = format!("e2e-claimed-{i}");
            let started = Instant::now();
            let outcome = serving.claim(&request(&sandbox_id)).await;
            let elapsed = millis(started.elapsed());
            match outcome {
                WarmOutcome::Claimed(_) => claim_ms.push(elapsed),
                WarmOutcome::Miss(m) => panic!("iteration {i}: expected claim, got miss: {m}"),
            }
            let view = provider
                .get_vm(&sandbox_id)
                .expect("query")
                .expect("claimed VM exists");
            assert_eq!(view.status, VmStatus::Running);
            provider.destroy_vm(&sandbox_id).expect("claimed destroy");
        }

        println!("== firecracker warm-pool e2e (n={N}, 1 vCPU / 256 MiB, file mem backend) ==");
        println!(
            "cold create->running ms: {cold_ms:.1?} median {:.1}",
            median(cold_ms.clone())
        );
        println!(
            "warm claim (acquire+rename+resume) ms: {claim_ms:.1?} median {:.1}",
            median(claim_ms.clone())
        );
        assert!(
            median(claim_ms.clone()) < median(cold_ms.clone()),
            "warm claim must beat cold boot (claim {:.1}ms vs cold {:.1}ms)",
            median(claim_ms),
            median(cold_ms)
        );
    }

    async fn wait_for_entry_nth<P: VmProvider + VmQuery + Clone + 'static>(
        serving: &WarmServing<P>,
        created_total_at_least: u64,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if serving.pool_metrics().created_total >= created_total_at_least {
                tokio::time::sleep(Duration::from_millis(100)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("warm pool never restored entry #{created_total_at_least}");
    }

    /// Probe: can a restore bind the vsock UDS recorded in the vmstate while
    /// the paused snapshot source still exists? Production relies on
    /// `prepare_snapshot_source` unlinking the socket file first; this pins
    /// that the unlink is sufficient (and documents Firecracker's raw
    /// behavior without it). Requires /dev/vhost-vsock in addition to the
    /// cold/claim prerequisites.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires /dev/kvm, /dev/vhost-vsock, firecracker binary, kernel + rootfs images"]
    async fn fc_warm_e2e_vsock_restore_binding() {
        use microvm_runtime::adapters::firecracker::FirecrackerVmProvider;
        use microvm_runtime::model::VsockSpec;

        assert!(
            std::path::Path::new("/dev/vhost-vsock").exists(),
            "e2e prerequisite missing: /dev/vhost-vsock (modprobe vhost_vsock)"
        );

        let tmp = tempfile::tempdir().expect("tempdir");
        let provider = FirecrackerVmProvider::new(e2e_config(tmp.path()));

        let uds = tmp.path().join("tpl-vsock.sock");
        let spec = VmSpec {
            vcpu_count: Some(1),
            mem_size_mib: Some(256),
            vsock: Some(VsockSpec {
                cid: 31337,
                uds_path: uds.clone(),
            }),
            ..VmSpec::default()
        };
        provider
            .create_vm_with_spec("vsock-tpl", &spec)
            .expect("template create");
        provider.start_vm("vsock-tpl").expect("template start");
        tokio::time::sleep(Duration::from_secs(2)).await;
        provider.stop_vm("vsock-tpl").expect("template pause");
        provider
            .snapshot_vm("vsock-tpl", GOLDEN_SNAPSHOT_ID)
            .expect("template snapshot");

        let restore_spec = || VmSpec {
            restore_from: Some(SnapshotRef {
                vm_id: "vsock-tpl".into(),
                snapshot_id: GOLDEN_SNAPSHOT_ID.into(),
                resume_immediately: false,
                network_overrides: vec![],
            }),
            ..VmSpec::default()
        };

        // Raw behavior: restore while the template's listener file exists.
        let raw = provider.create_vm_with_spec("vsock-entry-raw", &restore_spec());
        println!("restore WITH stale UDS file present: {raw:?}");
        if raw.is_ok() {
            provider
                .destroy_vm("vsock-entry-raw")
                .expect("destroy raw entry");
        }

        // Production behavior: prepare_snapshot_source unlinks the file.
        let _ = std::fs::remove_file(&uds);
        let prepared = provider.create_vm_with_spec("vsock-entry", &restore_spec());
        println!("restore AFTER unlinking template UDS: {prepared:?}");
        prepared.expect("restore must succeed once the UDS path is free");
        provider
            .rename_vm("vsock-entry", "vsock-claimed")
            .expect("rename restored entry");
        provider.start_vm("vsock-claimed").expect("resume claimed");

        provider
            .destroy_vm("vsock-claimed")
            .expect("destroy claimed");
        provider.destroy_vm("vsock-tpl").expect("destroy template");
    }

    #[test]
    fn warm_miss_display_names_every_reason() {
        let reasons = [
            WarmMiss::Disabled,
            WarmMiss::NotReady,
            WarmMiss::Empty,
            WarmMiss::DiskMismatch {
                requested: 1,
                pooled: 0,
            },
            WarmMiss::ImageMismatch {
                requested: "a".into(),
                pooled: "b".into(),
            },
            WarmMiss::CpuMismatch {
                requested: 4,
                pooled: 2,
            },
            WarmMiss::MemoryMismatch {
                requested: 2048,
                pooled: 1024,
            },
            WarmMiss::RenameFailed("x".into()),
            WarmMiss::ResumeFailed("y".into()),
        ];
        for reason in reasons {
            assert!(!reason.to_string().is_empty());
        }
    }
}
