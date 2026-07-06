//! The warm-serving engine: seed / refill / claim / retire.

use super::*;

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
