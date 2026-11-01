//! Durable warm-claim lineage.
//!
//! A claim holds the template's vsock CID, rootfs clone, and rider TAP, not
//! the sandbox id. The in-memory map dies on restart and `/proc` cannot see a
//! template already destroyed at claim, so delete would leak them.

use once_cell::sync::OnceCell;

use crate::error::Result;
use crate::firecracker_warm::WarmLineage;
use crate::store::{PersistentStore, state_dir};

static LINEAGE: OnceCell<PersistentStore<WarmLineage>> = OnceCell::new();

/// Durable store mapping `sandbox_id -> WarmLineage`. Mirrors
/// `runtime::sandboxes()`: lazily opened against `STORAGE_PATH`.
fn store() -> Result<&'static PersistentStore<WarmLineage>> {
    LINEAGE.get_or_try_init(|| PersistentStore::open(state_dir().join("fc-warm-lineage.json")))
}

/// Persist a claimed sandbox's lineage. Best-effort: a write failure degrades
/// to the pre-existing in-memory-only behavior (a restart-window leak), never
/// blocks the claim.
pub(crate) fn record(sandbox_id: &str, lineage: &WarmLineage) {
    match store().and_then(|s| s.insert(sandbox_id.to_string(), lineage.clone())) {
        Ok(()) => {}
        Err(err) => tracing::warn!(sandbox_id, %err, "failed to persist warm lineage"),
    }
}

/// Remove and return a sandbox's persisted lineage (called at delete).
pub(crate) fn take(sandbox_id: &str) -> Option<WarmLineage> {
    store()
        .ok()
        .and_then(|s| s.remove(sandbox_id).ok().flatten())
}

/// Template ids that a live sandbox's lineage still references — the set whose
/// `clones_dir` entries the reconcile sweep must preserve.
pub(crate) fn referenced_template_ids() -> Vec<String> {
    store()
        .ok()
        .and_then(|s| s.values().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.template_id)
        .collect()
}
