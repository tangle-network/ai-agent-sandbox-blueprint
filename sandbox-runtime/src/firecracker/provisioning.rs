//! VmSpec composition: network / vsock / rootfs attach, token mint, metadata inject.

use super::*;

/// Build the per-VM [`VmSpec`] from request-level resource overrides.
///
/// Pre-composition fields only; the [`NetworkInterface`] and [`VsockSpec`]
/// are appended by `attach_network_and_vsock` once they have been allocated,
/// and the optional per-VM rootfs path is set by `attach_rootfs` when a disk
/// resize is requested.
pub(crate) fn spec_from_request(req: &FirecrackerCreateRequest) -> VmSpec {
    let mut spec = VmSpec::default();
    if req.cpu_cores > 0 {
        // `cpu_cores` is `u64` on the sandbox side, but FC's `vcpu_count` is
        // `u8` (the kernel's max is 255). Clamp rather than overflow-panic.
        spec.vcpu_count = Some(req.cpu_cores.min(u8::MAX as u64) as u8);
    }
    if req.memory_mb > 0 {
        spec.mem_size_mib = Some(req.memory_mb.min(u32::MAX as u64) as u32);
    }
    spec
}

/// Allocate the host TAP + vsock CID for `vm_id`, augmenting `spec` with the
/// resulting network interface and vsock binding. Returns the [`VmNetwork`]
/// so the caller can build the host-reachable endpoint URL and the
/// per-VM UDS path so the post-boot metadata client can dial the guest.
///
/// This deliberately does NOT use [`microvm_runtime::FirecrackerComposer`]:
/// the composer hides the per-VM addressing from the caller (the `VmSpec`
/// it mutates is consumed inside the provider and never returned), and we
/// need the guest IP to build the endpoint URL.
pub(crate) fn attach_network_and_vsock(
    vm_id: &str,
    spec: &mut VmSpec,
) -> Result<(VmNetwork, std::path::PathBuf)> {
    let net = network();
    net.ensure_host()
        .map_err(|e| map_vm_error("ensure_host", vm_id, e))?;
    let vm_net = net
        .attach(vm_id)
        .map_err(|e| map_vm_error("network_attach", vm_id, e))?;
    let guest_mac = vm_net.mac_string();
    spec.network_interfaces.push(NetworkInterface {
        iface_id: "eth0".into(),
        host_dev_name: vm_net.tap_name.clone(),
        guest_mac: Some(guest_mac),
        rx_rate_limiter: None,
        tx_rate_limiter: None,
    });

    let vsock = vsock_manager();
    let vm_vsock = vsock
        .attach(vm_id)
        .map_err(|e| map_vm_error("vsock_attach", vm_id, e))?;
    vsock
        .ensure_uds_parent(&vm_vsock.uds_path)
        .map_err(|e| map_vm_error("vsock_ensure_uds_parent", vm_id, e))?;
    let uds_path = vm_vsock.uds_path.clone();
    spec.vsock = Some(VsockSpec {
        cid: vm_vsock.cid,
        uds_path: vm_vsock.uds_path,
    });

    Ok((vm_net, uds_path))
}

/// Resolve the requested stack name and, when one applies, clone it into a
/// per-VM rootfs slot sized to `disk_gb`. Returns `Ok(true)` when a clone
/// was performed (so `delete` knows to release it), `Ok(false)` when the
/// provider's default rootfs path was reused.
///
/// `disk_gb == 0` keeps the provider default regardless of `image`, matching
/// the historical behaviour where `disk_gb` was accepted for API parity.
pub(crate) fn attach_rootfs(
    vm_id: &str,
    req: &FirecrackerCreateRequest,
    spec: &mut VmSpec,
) -> Result<bool> {
    if req.disk_gb == 0 {
        return Ok(false);
    }
    let stack_name = if req.image.trim().is_empty() {
        match default_stack_name() {
            Some(s) => s,
            // No stack to clone from. Keep the provider default rootfs; the
            // caller asked for a size override we cannot honour without a
            // template, but failing the create here would be worse than
            // surfacing the (still-functional) default-size VM.
            None => return Ok(false),
        }
    } else {
        req.image.trim().to_string()
    };

    let target_bytes = req.disk_gb.saturating_mul(1024 * 1024 * 1024);
    let registry = rootfs_registry();
    let rootfs = registry
        .clone_for_vm_with_size(vm_id, &stack_name, target_bytes)
        .map_err(|e| map_vm_error("rootfs_clone", vm_id, e))?;
    spec.rootfs = Some(rootfs.path);
    Ok(true)
}

/// Mint a 32-byte sidecar auth token, URL-safe base64-encoded.
///
/// The token is opaque to the host past this point — it is pushed verbatim
/// into the guest secrets directory and returned to the caller so the
/// sandbox record can stamp the same value the sidecar will compare against.
pub(crate) fn mint_sidecar_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    // Match `auth::generate_token`'s hex encoding so downstream consumers
    // (sidecar comparators, log redactors) see a single token format.
    hex::encode(bytes)
}

/// Push the env table and the sidecar auth token into the guest via the
/// metadata service. Returns the minted token on success.
///
/// The connection takes a moment to come up after `start_vm` (cold boot is
/// 1-3s, snapshot restore <100ms); [`GuestMetadataClient::connect`] handles
/// the retry loop against `connect_timeout`. The whole transaction runs on
/// a blocking worker because the underlying UnixStream calls block.
pub(crate) async fn inject_runtime_metadata(
    vm_id: &str,
    uds_path: std::path::PathBuf,
    env: HashMap<String, String>,
) -> Result<String> {
    let token = mint_sidecar_token();
    let token_bytes = token.clone().into_bytes();
    let vm_id_owned = vm_id.to_string();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<(), VmRuntimeError> {
        let client = GuestMetadataClient::new(uds_path, GuestMetadataConfig::from_env());
        let mut conn = client.connect()?;
        if !env.is_empty() {
            conn.set_env(&env)?;
        }
        conn.set_secret(SIDECAR_AUTH_TOKEN_SECRET, &token_bytes)?;
        Ok(())
    })
    .await
    .map_err(|e| {
        SandboxError::Unavailable(format!("guest metadata join error for {vm_id_owned}: {e}"))
    })?;
    result.map_err(|e| map_vm_error("guest_metadata", vm_id, e))?;
    Ok(token)
}

/// Tear down the host-side allocations associated with `vm_id`. Best-effort:
/// individual failures are logged via `tracing` but never propagated. Used
/// both on the create-failure cleanup path and on `delete`.
pub(crate) fn release_attachments(vm_id: &str, attachments: &VmAttachments) {
    if attachments.dnat_rule_count > 0
        && let Err(err) = firecracker_dnat::release_port_forwards(vm_id)
    {
        tracing::warn!(vm_id, %err, "failed to release firecracker DNAT rules");
    }
    if attachments.vsock_attached
        && let Err(err) = vsock_manager().detach(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to detach firecracker vsock");
    }
    if attachments.network_attached
        && let Err(err) = network().detach(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to detach firecracker network");
    }
    if attachments.rootfs_cloned
        && let Err(err) = rootfs_registry().release(vm_id)
    {
        tracing::warn!(vm_id, ?err, "failed to release firecracker rootfs clone");
    }
    if let Some(warm) = &attachments.warm {
        // Warm-claimed sandboxes hold host resources under lineage ids: the
        // vsock CID (and, when cloned, the rootfs slot) live under the
        // template id; the TAP the VM rides lives under the rider id. All
        // are single-owner by construction (one claim per generation), so
        // releasing them here cannot race another VM.
        if warm.rootfs_cloned
            && let Err(err) = rootfs_registry().release(&warm.template_id)
        {
            tracing::warn!(
                vm_id,
                template_id = %warm.template_id,
                ?err,
                "failed to release warm template rootfs clone"
            );
        }
        if let Err(err) = vsock_manager().detach(&warm.template_id) {
            tracing::warn!(
                vm_id,
                template_id = %warm.template_id,
                ?err,
                "failed to detach warm template vsock"
            );
        }
        if let Some(rider_id) = &warm.rider_id
            && let Err(err) = network().detach(rider_id)
        {
            tracing::warn!(vm_id, rider_id, ?err, "failed to detach warm rider network");
        }
    }
}
