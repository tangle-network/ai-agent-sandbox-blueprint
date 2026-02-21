//! Persistent provision progress tracking for sandbox creation.
//!
//! Operators can expose this via an API so frontends can poll creation status
//! in real-time rather than waiting for the full provision to complete.
//!
//! Progress is persisted to disk so it survives operator restarts and can be
//! queried by external systems. The `metadata` field allows blueprint-specific
//! data (e.g. `service_id`, `bot_id`) without modifying the core schema.

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SandboxError};
use crate::store::PersistentStore;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionPhase {
    Queued,
    ImagePull,
    ContainerCreate,
    ContainerStart,
    HealthCheck,
    Ready,
    Failed,
}

impl ProvisionPhase {
    /// Progress percentage (0–100) for UI rendering.
    pub fn progress_pct(self) -> u8 {
        match self {
            Self::Queued => 0,
            Self::ImagePull => 20,
            Self::ContainerCreate => 40,
            Self::ContainerStart => 60,
            Self::HealthCheck => 80,
            Self::Ready => 100,
            Self::Failed => 0,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Failed)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvisionStatus {
    pub call_id: u64,
    pub sandbox_id: Option<String>,
    pub phase: ProvisionPhase,
    pub message: Option<String>,
    pub started_at: u64,
    pub updated_at: u64,
    pub progress_pct: u8,
    /// Sidecar URL populated when the provision reaches the Ready phase.
    #[serde(default)]
    pub sidecar_url: Option<String>,
    /// Blueprint-specific metadata (service_id, bot_id, etc.).
    /// Defaults to `null` for backward compatibility.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Persistent store
// ---------------------------------------------------------------------------

static PROVISIONS: OnceCell<PersistentStore<ProvisionStatus>> = OnceCell::new();

/// Access the provision progress persistent store.
pub fn provisions() -> Result<&'static PersistentStore<ProvisionStatus>> {
    PROVISIONS
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("provisions.json");
            PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

/// Begin tracking a new provision for the given call ID.
pub fn start_provision(call_id: u64) -> Result<ProvisionStatus> {
    let now = crate::util::now_ts();
    let status = ProvisionStatus {
        call_id,
        sandbox_id: None,
        phase: ProvisionPhase::Queued,
        message: Some("Queued for provisioning".into()),
        started_at: now,
        updated_at: now,
        progress_pct: 0,
        sidecar_url: None,
        metadata: serde_json::Value::Null,
    };
    provisions()?.insert(call_id.to_string(), status.clone())?;
    Ok(status)
}

/// Update the provision phase for a call. Returns the updated status.
pub fn update_provision(
    call_id: u64,
    phase: ProvisionPhase,
    message: Option<String>,
    sandbox_id: Option<String>,
    sidecar_url: Option<String>,
) -> Result<Option<ProvisionStatus>> {
    let now = crate::util::now_ts();
    let key = call_id.to_string();
    let store = provisions()?;

    let updated = store.update(&key, |entry| {
        entry.phase = phase;
        entry.progress_pct = phase.progress_pct();
        entry.updated_at = now;
        if let Some(msg) = message {
            entry.message = Some(msg);
        }
        if let Some(id) = sandbox_id {
            entry.sandbox_id = Some(id);
        }
        if let Some(url) = sidecar_url {
            entry.sidecar_url = Some(url);
        }
    })?;

    if updated {
        Ok(store.get(&key)?)
    } else {
        Ok(None)
    }
}

/// Update the metadata for a provision.
pub fn update_provision_metadata(
    call_id: u64,
    metadata: serde_json::Value,
) -> Result<bool> {
    let key = call_id.to_string();
    provisions()?.update(&key, |entry| {
        entry.metadata = metadata;
    })
}

/// Get the current provision status for a call.
pub fn get_provision(call_id: u64) -> Result<Option<ProvisionStatus>> {
    provisions()?.get(&call_id.to_string())
}

/// List all active (non-terminal) provisions.
pub fn list_active_provisions() -> Result<Vec<ProvisionStatus>> {
    Ok(provisions()?
        .values()?
        .into_iter()
        .filter(|s| !s.phase.is_terminal())
        .collect())
}

/// List all provisions (including completed/failed).
pub fn list_all_provisions() -> Result<Vec<ProvisionStatus>> {
    provisions()?.values()
}

/// Remove terminal provisions older than `max_age_secs`.
pub fn gc_provisions(max_age_secs: u64) -> Result<()> {
    let cutoff = crate::util::now_ts().saturating_sub(max_age_secs);
    let store = provisions()?;
    let to_remove: Vec<String> = store
        .values()?
        .into_iter()
        .filter(|s| s.phase.is_terminal() && s.updated_at <= cutoff)
        .map(|s| s.call_id.to_string())
        .collect();

    for key in to_remove {
        store.remove(&key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();
    fn init() {
        INIT.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("provision-progress-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", dir) };
        });
    }

    #[test]
    fn provision_lifecycle() {
        init();

        let call_id = 42_000_001; // unique to avoid test collisions
        let status = start_provision(call_id).unwrap();
        assert_eq!(status.phase, ProvisionPhase::Queued);
        assert_eq!(status.progress_pct, 0);
        assert_eq!(status.metadata, serde_json::Value::Null);

        let updated = update_provision(
            call_id,
            ProvisionPhase::ImagePull,
            Some("Pulling image".into()),
            None,
            None,
        )
        .unwrap();
        assert!(updated.is_some());
        let updated = updated.unwrap();
        assert_eq!(updated.phase, ProvisionPhase::ImagePull);
        assert_eq!(updated.progress_pct, 20);

        let updated = update_provision(
            call_id,
            ProvisionPhase::Ready,
            Some("Sandbox ready".into()),
            Some("sandbox-abc".into()),
            Some("http://localhost:3000".into()),
        )
        .unwrap();
        let updated = updated.unwrap();
        assert_eq!(updated.phase, ProvisionPhase::Ready);
        assert_eq!(updated.progress_pct, 100);
        assert_eq!(updated.sandbox_id.as_deref(), Some("sandbox-abc"));

        // Should be retrievable
        let fetched = get_provision(call_id).unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().phase, ProvisionPhase::Ready);

        // Our provision should not appear in active list (Ready is terminal)
        let active = list_active_provisions().unwrap();
        assert!(!active.iter().any(|s| s.call_id == call_id));
    }

    #[test]
    fn provision_metadata() {
        init();

        let call_id = 42_000_002;
        start_provision(call_id).unwrap();

        let meta = serde_json::json!({ "service_id": 123, "bot_id": "bot-abc" });
        update_provision_metadata(call_id, meta.clone()).unwrap();

        let fetched = get_provision(call_id).unwrap().unwrap();
        assert_eq!(fetched.metadata, meta);
    }
}
