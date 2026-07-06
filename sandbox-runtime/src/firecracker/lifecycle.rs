//! Sandbox lifecycle: health, create_and_start, start, stop, delete, status.

use super::*;

/// Sanity-check the configured Firecracker provider is reachable.
///
/// Used by `GET /health` on the operator API. The primitive initialises
/// lazily from env vars, so this confirms it constructs without panicking;
/// real prereq validation (kernel / rootfs / binary present) happens at
/// `create_vm` time and would surface there with an [`SandboxError::Unavailable`].
pub(crate) async fn health() -> Result<()> {
    let _ = provider();
    Ok(())
}

/// Create a VM and start it via the in-process driver, attaching network,
/// vsock, rootfs (when sized), and any requested host port forwards; then
/// inject per-VM env and the sidecar auth token into the guest via the
/// metadata service before returning.
///
/// On any failure after VM creation, the VM is destroyed and all attached
/// host resources are released before returning. On success, a record of
/// the attachments is stored so a later `delete` releases them.
pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    // Warm-pool serving first (SANDBOX_FC_WARM_POOL_SIZE > 0): a claim
    // hands off a pre-restored VM via rename + resume. Any miss is logged
    // with its typed reason and falls through to the cold boot below — the
    // one designed fallback in this module. Invalid warm configuration is a
    // hard error, never a silent cold boot.
    match warm_claim(&req).await? {
        WarmOutcome::Claimed(claim) => return finish_warm_claim(req, claim).await,
        WarmOutcome::Miss(firecracker_warm::WarmMiss::Disabled) => {}
        WarmOutcome::Miss(miss) => {
            tracing::info!(
                sandbox_id = %req.session_id,
                reason = %miss,
                "firecracker warm-pool miss; falling back to cold boot"
            );
        }
    }

    let vm_id = req.session_id.clone();
    let mut spec = spec_from_request(&req);

    // Compose the per-VM TAP + vsock binding pre-spawn. The returned
    // `VmNetwork` carries the guest IP that the sidecar endpoint URL is
    // built from; the UDS path is used post-boot to push env + secrets.
    let (vm_net, vsock_uds_path) = match attach_network_and_vsock(&vm_id, &mut spec) {
        Ok(v) => v,
        Err(err) => {
            // ensure_host failed, or attach failed mid-way — release whatever
            // partial state may have been created. Both managers are
            // idempotent under "nothing to release".
            let _ = network().detach(&vm_id);
            let _ = vsock_manager().detach(&vm_id);
            return Err(err);
        }
    };

    // Per-VM disk sizing via the rootfs registry. Skipped entirely when
    // `disk_gb == 0` so the provider's default rootfs is reused untouched.
    let rootfs_cloned = match attach_rootfs(&vm_id, &req, &mut spec) {
        Ok(v) => v,
        Err(err) => {
            release_attachments(&vm_id, &VmAttachments::cold(0, false));
            return Err(err);
        }
    };

    // The blocking lifecycle calls on `FirecrackerVmProvider` shell out to a
    // child process and do unix-socket I/O; isolate them onto a blocking
    // worker so the async runtime stays responsive.
    let create_id = vm_id.clone();
    let create_spec = spec.clone();
    if let Err(err) = tokio::task::spawn_blocking(move || {
        provider().create_vm_with_spec(&create_id, &create_spec)
    })
    .await
    .map_err(|e| {
        SandboxError::Unavailable(format!("firecracker create join error for {vm_id}: {e}"))
    })?
    .map_err(|e| map_vm_error("create", &vm_id, e))
    {
        release_attachments(&vm_id, &VmAttachments::cold(0, rootfs_cloned));
        return Err(err);
    }

    let start_id = vm_id.clone();
    if let Err(err) = tokio::task::spawn_blocking(move || provider().start_vm(&start_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker start join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("start", &vm_id, e))
    {
        // Best-effort cleanup so a partial create doesn't leak a process.
        let cleanup_id = vm_id.clone();
        let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
        release_attachments(&vm_id, &VmAttachments::cold(0, rootfs_cloned));
        return Err(err);
    }

    // Install DNAT for each requested port forward. The host doesn't know
    // its own public IP in this context — we DNAT on the wildcard host
    // address and let the kernel match traffic on the egress interface.
    let mut dnat_rule_count = 0usize;
    for port in &req.ports {
        if let Err(err) = firecracker_dnat::install_port_forward(&vm_id, vm_net.guest_ip, *port) {
            tracing::error!(
                vm_id = %vm_id,
                host_port = port.host_port,
                container_port = port.container_port,
                %err,
                "failed to install DNAT for firecracker port forward; tearing down"
            );
            // Tear everything down.
            let cleanup_id = vm_id.clone();
            let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
            release_attachments(&vm_id, &VmAttachments::cold(dnat_rule_count, rootfs_cloned));
            return Err(SandboxError::Unavailable(format!(
                "firecracker port forward install failed for {vm_id}: {err}"
            )));
        }
        dnat_rule_count += 1;
    }

    // Inject per-VM env + sidecar auth token into the guest. Any failure
    // here is a real runtime error (the guest daemon is unreachable, the
    // env is malformed, etc.) — propagate it after rolling back the VM so
    // the caller does not end up with an inaccessible sandbox.
    let sidecar_auth_token = match inject_runtime_metadata(&vm_id, vsock_uds_path, req.env).await {
        Ok(t) => t,
        Err(err) => {
            let cleanup_id = vm_id.clone();
            let _ = tokio::task::spawn_blocking(move || provider().destroy_vm(&cleanup_id)).await;
            release_attachments(&vm_id, &VmAttachments::cold(dnat_rule_count, rootfs_cloned));
            return Err(err);
        }
    };

    record_attachments(&vm_id, VmAttachments::cold(dnat_rule_count, rootfs_cloned));

    let endpoint = format!("http://{}:{}", vm_net.guest_ip, sidecar_port());

    Ok(FirecrackerProvisionResult {
        container: FirecrackerContainer {
            id: vm_id,
            endpoint: Some(endpoint),
        },
        sidecar_auth_token: Some(sidecar_auth_token),
    })
}

/// Resume a previously-stopped VM.
///
/// Used by the sandbox `resume` lifecycle. The host attachments (TAP, CID,
/// DNAT) survive across stop/start, so resuming a known VM only needs to
/// drive the primitive's lifecycle and rebuild the endpoint from the
/// recorded `VmNetwork`-equivalent allocation.
pub(crate) async fn start(container_id: &str) -> Result<FirecrackerContainer> {
    let vm_id = container_id.to_string();
    let start_id = vm_id.clone();
    tokio::task::spawn_blocking(move || provider().start_vm(&start_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker start join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("start", &vm_id, e))?;

    // Warm-claimed VMs keep the guest IP baked into their template's
    // snapshot; deriving one from the sandbox id here would report an
    // endpoint the guest never configured.
    let guest_ip = match warm_guest_ip(container_id) {
        Some(ip) => ip,
        None => {
            // `NetworkManager::attach` is idempotent: for a known vm_id it
            // returns the same TAP / IP allocation. Resuming a VM whose TAP
            // was torn down out-of-band recreates it; this matches what an
            // operator would do by hand for debugging.
            network()
                .attach(container_id)
                .map_err(|e| map_vm_error("network_attach_resume", container_id, e))?
                .guest_ip
        }
    };
    let endpoint = format!("http://{}:{}", guest_ip, sidecar_port());

    Ok(FirecrackerContainer {
        id: container_id.to_string(),
        endpoint: Some(endpoint),
    })
}

/// Stop a running VM.
///
/// Idempotent for callers: a missing VM (already torn down) is treated as
/// success because the reaper reconcile path needs to be able to call this
/// without first checking existence. Other lifecycle errors surface as
/// [`SandboxError::Validation`].
pub(crate) async fn stop(container_id: &str) -> Result<()> {
    let vm_id = container_id.to_string();
    let stop_id = vm_id.clone();
    match tokio::task::spawn_blocking(move || provider().stop_vm(&stop_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker stop join error for {vm_id}: {e}"))
        })? {
        Ok(()) => Ok(()),
        Err(VmRuntimeError::VmNotFound(_)) => Ok(()),
        Err(VmRuntimeError::InvalidTransition { .. }) => {
            // Already stopped — treat as idempotent success.
            Ok(())
        }
        Err(err) => Err(map_vm_error("stop", &vm_id, err)),
    }
}

/// Destroy a VM permanently and release all per-VM host attachments
/// (DNAT rules, vsock CID, TAP, rootfs clone).
///
/// Idempotent: missing or already-destroyed VMs return `Ok(())` so callers
/// can treat delete-after-delete as a no-op. Attachment release runs even
/// if the VMM tear-down errors out — leaving the host with orphan iptables
/// rules or TAPs is worse than swallowing a redundant destroy error.
pub(crate) async fn delete(container_id: &str) -> Result<()> {
    let vm_id = container_id.to_string();

    let destroy_id = vm_id.clone();
    let destroy_outcome = tokio::task::spawn_blocking(move || provider().destroy_vm(&destroy_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("firecracker destroy join error for {vm_id}: {e}"))
        })?;

    // Release host-side allocations whether or not destroy succeeded — the
    // VM is going away either way and orphan resources are worse than
    // double-released ones (every release path is idempotent).
    // Clear the durable lineage entry regardless; it is released either through
    // the in-memory attachments (normal delete) or reconstructed below.
    let persisted_lineage = crate::firecracker_lineage::take(&vm_id);
    if let Some(attachments) = take_attachments(&vm_id) {
        release_attachments(&vm_id, &attachments);
    } else {
        // The operator restarted and lost the in-memory attachment map. Release
        // the sandbox's own-id resources (best-effort, all idempotent) plus the
        // durably-persisted warm lineage — the template's rootfs clone + vsock
        // CID and the rider TAP, which live under ids other than this sandbox's
        // and would otherwise leak.
        release_attachments(
            &vm_id,
            &VmAttachments {
                network_attached: true,
                vsock_attached: true,
                dnat_rule_count: 1,
                rootfs_cloned: true,
                warm: persisted_lineage,
            },
        );
    }

    match destroy_outcome {
        Ok(()) => Ok(()),
        Err(VmRuntimeError::VmNotFound(_)) => Ok(()),
        Err(VmRuntimeError::InvalidTransition { .. }) => Ok(()),
        Err(err) => Err(map_vm_error("destroy", &vm_id, err)),
    }
}

/// Inspect lifecycle status for the reaper reconcile loop.
///
/// Maps [`VmStatus`] onto the sandbox-facing tri-state. A `None` result from
/// the primitive (the VM is absent from the provider's view) collapses to
/// `Missing`, which signals the reaper to remove the orphan record.
pub(crate) async fn status(container_id: &str) -> Result<FirecrackerContainerStatus> {
    let vm_id = container_id.to_string();
    let view = tokio::task::spawn_blocking(move || provider().get_vm(&vm_id))
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!(
                "firecracker status join error for {container_id}: {e}"
            ))
        })?
        .map_err(|e| map_vm_error("status", container_id, e))?;

    Ok(match view.map(|v| v.status) {
        None => FirecrackerContainerStatus::Missing,
        Some(VmStatus::Running) => FirecrackerContainerStatus::Running,
        Some(VmStatus::Created) | Some(VmStatus::Stopped) => FirecrackerContainerStatus::Stopped,
        Some(VmStatus::Destroyed) => FirecrackerContainerStatus::Missing,
    })
}
