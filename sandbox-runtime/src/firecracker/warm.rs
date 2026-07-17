//! Warm-serving glue: engine handle, claim, and claim finalization.

use super::*;

/// Process-wide warm-serving engine. `None` until the first create with
/// `SANDBOX_FC_WARM_POOL_SIZE > 0` — constructing it spins up the pool's
/// refill thread, which a disabled deployment must never pay for.
pub(crate) static WARM_SERVING: OnceLock<Arc<WarmServing<FirecrackerVmProvider>>> = OnceLock::new();

/// Whether the warm engine was ever constructed. Test-visible so the
/// admission-order invariant ("a rejected create never touches the pool")
/// can be pinned from integration tests.
#[cfg(any(test, feature = "test-utils"))]
pub fn warm_pool_initialized_for_tests() -> bool {
    WARM_SERVING.get().is_some()
}

/// Drive [`reconcile_warm_orphans`] from an integration test.
#[cfg(any(test, feature = "test-utils"))]
pub fn reconcile_warm_orphans_for_tests() {
    reconcile_warm_orphans();
}

/// Try to serve the create from the warm pool. Returns the typed outcome;
/// the caller logs misses and proceeds with the cold path.
///
/// MUST only be called from [`create_and_start`]: the runtime layer runs
/// `admit_sandbox_resources` (per-sandbox maxima + the single-pass store
/// admission covering the count cap and memory budget, under the creation
/// permit) before dispatching here, which is what makes a warm claim count
/// against `SANDBOX_MAX_COUNT` / the host memory budget exactly like a
/// cold boot. Pool inventory itself is not a sandbox and is never
/// admission-accounted — see the invariant note in [`firecracker_warm`].
pub(crate) async fn warm_claim(req: &FirecrackerCreateRequest) -> Result<WarmOutcome> {
    let pool_size = firecracker_warm::configured_pool_size()?;
    if pool_size == 0 {
        return Ok(WarmOutcome::Miss(firecracker_warm::WarmMiss::Disabled));
    }

    let serving = match WARM_SERVING.get() {
        Some(s) => s,
        None => {
            let entry_max_age = firecracker_warm::configured_entry_max_age()?;
            let disk_gb = firecracker_warm::configured_warm_disk_gb()?;
            // Workspace defaults define the pooled machine shape; requests
            // must match them (or leave cpu/mem unset) to claim.
            let fc_config = FirecrackerConfig::from_env();
            let settings = WarmSettings {
                pool_size,
                stack: default_stack_name(),
                disk_gb,
                vcpu_count: fc_config.vcpu_count,
                mem_size_mib: fc_config.mem_size_mib,
                entry_max_age,
            };
            WARM_SERVING.get_or_init(|| {
                // Reap warm VMs orphaned by a previous operator process BEFORE
                // seeding: a fresh process resets the generation counter to 0
                // and re-mints the same ids, so reaping after the first seed
                // would kill the just-seeded generation.
                reconcile_warm_orphans();
                Arc::new(WarmServing::new(
                    provider().clone(),
                    Arc::new(OperatorWarmHost),
                    settings,
                ))
            })
        }
    };

    serving.ensure_seeding();
    Ok(serving
        .claim(&WarmClaimRequest {
            sandbox_id: req.session_id.clone(),
            image: req.image.clone(),
            cpu_cores: req.cpu_cores,
            memory_mb: req.memory_mb,
            disk_gb: req.disk_gb,
        })
        .await)
}

/// Finish provisioning a warm-claimed VM: DNAT for requested ports, per-VM
/// env + sidecar token over the inherited vsock, attachment bookkeeping,
/// endpoint from the inherited guest IP. Any failure destroys the claimed
/// VM and propagates — the claim already consumed the generation, and the
/// same environmental cause would fail a cold boot's identical steps.
pub(crate) async fn finish_warm_claim(
    req: FirecrackerCreateRequest,
    claim: WarmClaim,
) -> Result<FirecrackerProvisionResult> {
    let vm_id = req.session_id.clone();
    let guest_ip: Ipv4Addr = claim.guest_ip.ok_or_else(|| {
        SandboxError::Unavailable(format!(
            "warm claim for {vm_id} carried no guest IP; the operator warm host always \
             composes one — this indicates a non-production WarmHost in a production path"
        ))
    })?;
    let metadata_uds = claim.metadata_uds.clone().ok_or_else(|| {
        SandboxError::Unavailable(format!(
            "warm claim for {vm_id} carried no metadata UDS; env injection is impossible"
        ))
    })?;

    async fn teardown_warm_claim(vm_id: &str, dnat_rule_count: usize, lineage: WarmLineage) {
        let cleanup_id = vm_id.to_string();
        let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
        release_attachments(
            vm_id,
            &VmAttachments {
                network_attached: false,
                vsock_attached: false,
                dnat_rule_count,
                rootfs_cloned: false,
                warm: Some(lineage),
            },
        );
    }

    let mut dnat_rule_count = 0usize;
    for port in &req.ports {
        if let Err(err) = firecracker_dnat::install_port_forward(&vm_id, guest_ip, *port) {
            tracing::error!(
                vm_id = %vm_id,
                host_port = port.host_port,
                container_port = port.container_port,
                %err,
                "failed to install DNAT for warm-claimed port forward; tearing down"
            );
            teardown_warm_claim(&vm_id, dnat_rule_count, claim.lineage.clone()).await;
            return Err(SandboxError::Unavailable(format!(
                "firecracker port forward install failed for warm-claimed {vm_id}: {err}"
            )));
        }
        dnat_rule_count += 1;
    }

    let sidecar_auth_token =
        match inject_runtime_metadata(&vm_id, metadata_uds, req.env.clone()).await {
            Ok(t) => t,
            Err(err) => {
                teardown_warm_claim(&vm_id, dnat_rule_count, claim.lineage.clone()).await;
                return Err(err);
            }
        };

    // Persist the lineage durably so a delete or reconcile after an operator
    // restart (which loses the in-memory attachment map) can still release the
    // template's rootfs clone + vsock CID and the rider TAP.
    crate::firecracker_lineage::record(&vm_id, &claim.lineage);

    record_attachments(
        &vm_id,
        VmAttachments {
            network_attached: false,
            vsock_attached: false,
            dnat_rule_count,
            rootfs_cloned: false,
            warm: Some(claim.lineage),
        },
    );

    let endpoint = format!("http://{guest_ip}:{}", sidecar_port());
    Ok(FirecrackerProvisionResult {
        container: FirecrackerContainer {
            id: vm_id,
            endpoint: Some(endpoint),
        },
        sidecar_auth_token: Some(sidecar_auth_token),
    })
}

/// Guest IP a warm-claimed sandbox inherited from its template, if any.
/// Consulted by [`start`] so resume rebuilds the endpoint from the IP the
/// guest actually has instead of the sandbox-id-derived allocation.
pub(crate) fn warm_guest_ip(vm_id: &str) -> Option<Ipv4Addr> {
    attachments_map()
        .lock()
        .ok()?
        .get(vm_id)?
        .warm
        .as_ref()?
        .guest_ip
}
