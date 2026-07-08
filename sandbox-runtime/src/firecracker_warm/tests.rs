//! Test-support host + fixtures + the firecracker_warm unit tests.

use super::*;

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
mod warm_serving {
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
