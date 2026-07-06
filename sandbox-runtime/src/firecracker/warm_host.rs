//! Operator WarmHost: real network / vsock / rootfs composition for the warm pool.

use super::*;

/// Production [`WarmHost`]: composes template identity with the same
/// primitives the cold path uses (`NetworkManager` / `VsockManager` /
/// `RootfsRegistry`) and gates template readiness on the guest metadata
/// daemon answering over vsock — the identical readiness bar a cold-booted
/// sandbox must clear before env injection.
pub(crate) struct OperatorWarmHost;

#[async_trait]
impl WarmHost for OperatorWarmHost {
    async fn compose_template(
        &self,
        template_id: &str,
        disk_gb: u64,
        stack: Option<&str>,
        spec: &mut VmSpec,
    ) -> Result<TemplateIdentity> {
        let (vm_net, uds_path) = match attach_network_and_vsock(template_id, spec) {
            Ok(v) => v,
            Err(err) => {
                let _ = network().detach(template_id);
                let _ = vsock_manager().detach(template_id);
                return Err(err);
            }
        };

        let mut rootfs_cloned = false;
        if disk_gb > 0 {
            let stack_name = stack.ok_or_else(|| {
                SandboxError::Validation(
                    "SANDBOX_FC_WARM_DISK_GB > 0 requires SANDBOX_FIRECRACKER_DEFAULT_STACK \
                     so the warm template has a rootfs template to clone"
                        .to_string(),
                )
            })?;
            let target_bytes = disk_gb.saturating_mul(1024 * 1024 * 1024);
            match rootfs_registry().clone_for_vm_with_size(template_id, stack_name, target_bytes) {
                Ok(rootfs) => {
                    spec.rootfs = Some(rootfs.path);
                    rootfs_cloned = true;
                }
                Err(err) => {
                    let _ = network().detach(template_id);
                    let _ = vsock_manager().detach(template_id);
                    return Err(map_vm_error("warm rootfs_clone", template_id, err));
                }
            }
        }

        Ok(TemplateIdentity {
            guest_ip: Some(vm_net.guest_ip),
            metadata_uds: Some(uds_path),
            rootfs_cloned,
        })
    }

    async fn await_guest_ready(
        &self,
        template_id: &str,
        identity: &TemplateIdentity,
    ) -> Result<()> {
        let uds_path = identity.metadata_uds.clone().ok_or_else(|| {
            SandboxError::Unavailable(format!(
                "warm template {template_id} composed without a vsock UDS"
            ))
        })?;
        let vm_id = template_id.to_string();
        tokio::task::spawn_blocking(move || -> std::result::Result<(), VmRuntimeError> {
            let client = GuestMetadataClient::new(uds_path, GuestMetadataConfig::from_env());
            let mut conn = client.connect()?;
            conn.ping()
        })
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("warm guest-ready join error for {vm_id}: {e}"))
        })?
        .map_err(|e| map_vm_error("warm guest_ready", template_id, e))
    }

    async fn prepare_snapshot_source(&self, template_id: &str, identity: &TemplateIdentity) {
        // The vmstate records the template's vsock UDS path; the restored
        // entry's Firecracker binds a fresh listener there. The paused
        // template still holds the old listener fd — unlinking the file
        // orphans it (the template never serves again; it exists only as
        // the snapshot's durable home) and frees the path for the entry.
        if let Some(uds) = &identity.metadata_uds
            && let Err(err) = std::fs::remove_file(uds)
        {
            tracing::warn!(
                template_id,
                uds = %uds.display(),
                %err,
                "failed to unlink warm template vsock UDS; entry restore may fail to bind"
            );
        }
    }

    async fn attach_rider(&self, rider_id: &str) -> Result<Option<NetworkInterface>> {
        // The rider TAP carries the claimed VM's traffic; the guest keeps
        // the MAC + IP recorded in the snapshot (Firecracker's restore
        // override swaps the host device only), so no guest_mac is set.
        let vm_net = network()
            .attach(rider_id)
            .map_err(|e| map_vm_error("warm rider network_attach", rider_id, e))?;
        Ok(Some(NetworkInterface {
            iface_id: "eth0".into(),
            host_dev_name: vm_net.tap_name,
            guest_mac: None,
            rx_rate_limiter: None,
            tx_rate_limiter: None,
        }))
    }

    async fn release_template(&self, template_id: &str, identity: &TemplateIdentity, all: bool) {
        if let Err(err) = network().detach(template_id) {
            tracing::warn!(template_id, ?err, "failed to detach warm template network");
        }
        if all {
            if let Err(err) = vsock_manager().detach(template_id) {
                tracing::warn!(template_id, ?err, "failed to detach warm template vsock");
            }
            if identity.rootfs_cloned
                && let Err(err) = rootfs_registry().release(template_id)
            {
                tracing::warn!(
                    template_id,
                    ?err,
                    "failed to release warm template rootfs clone"
                );
            }
        }
    }
}
