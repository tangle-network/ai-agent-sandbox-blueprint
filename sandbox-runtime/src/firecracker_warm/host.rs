//! Host-side network/vsock/rootfs composition trait.

use super::*;

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
