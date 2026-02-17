//! In-memory provision progress tracking for sandbox creation.
//!
//! Operators can expose this via an API so frontends can poll creation status
//! in real-time rather than waiting for the full provision to complete.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

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
}

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

static PROVISIONS: Lazy<Mutex<HashMap<u64, ProvisionStatus>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Begin tracking a new provision for the given call ID.
pub fn start_provision(call_id: u64) -> ProvisionStatus {
    let now = crate::util::now_ts();
    let status = ProvisionStatus {
        call_id,
        sandbox_id: None,
        phase: ProvisionPhase::Queued,
        message: Some("Queued for provisioning".into()),
        started_at: now,
        updated_at: now,
        progress_pct: 0,
    };
    PROVISIONS
        .lock()
        .unwrap()
        .insert(call_id, status.clone());
    status
}

/// Update the provision phase for a call. Returns the updated status.
pub fn update_provision(
    call_id: u64,
    phase: ProvisionPhase,
    message: Option<String>,
    sandbox_id: Option<String>,
) -> Option<ProvisionStatus> {
    let now = crate::util::now_ts();
    let mut map = PROVISIONS.lock().unwrap();
    let entry = map.get_mut(&call_id)?;
    entry.phase = phase;
    entry.progress_pct = phase.progress_pct();
    entry.updated_at = now;
    if let Some(msg) = message {
        entry.message = Some(msg);
    }
    if let Some(id) = sandbox_id {
        entry.sandbox_id = Some(id);
    }
    Some(entry.clone())
}

/// Get the current provision status for a call.
pub fn get_provision(call_id: u64) -> Option<ProvisionStatus> {
    PROVISIONS.lock().unwrap().get(&call_id).cloned()
}

/// List all active (non-terminal) provisions.
pub fn list_active_provisions() -> Vec<ProvisionStatus> {
    PROVISIONS
        .lock()
        .unwrap()
        .values()
        .filter(|s| !s.phase.is_terminal())
        .cloned()
        .collect()
}

/// List all provisions (including completed/failed).
pub fn list_all_provisions() -> Vec<ProvisionStatus> {
    PROVISIONS.lock().unwrap().values().cloned().collect()
}

/// Remove terminal provisions older than `max_age_secs`.
pub fn gc_provisions(max_age_secs: u64) {
    let cutoff = crate::util::now_ts().saturating_sub(max_age_secs);
    PROVISIONS.lock().unwrap().retain(|_, s| {
        !s.phase.is_terminal() || s.updated_at > cutoff
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_lifecycle() {
        let call_id = 42;
        let status = start_provision(call_id);
        assert_eq!(status.phase, ProvisionPhase::Queued);
        assert_eq!(status.progress_pct, 0);

        let updated = update_provision(
            call_id,
            ProvisionPhase::ImagePull,
            Some("Pulling image".into()),
            None,
        );
        assert!(updated.is_some());
        let updated = updated.unwrap();
        assert_eq!(updated.phase, ProvisionPhase::ImagePull);
        assert_eq!(updated.progress_pct, 20);

        let updated = update_provision(
            call_id,
            ProvisionPhase::Ready,
            Some("Sandbox ready".into()),
            Some("sandbox-abc".into()),
        );
        let updated = updated.unwrap();
        assert_eq!(updated.phase, ProvisionPhase::Ready);
        assert_eq!(updated.progress_pct, 100);
        assert_eq!(updated.sandbox_id.as_deref(), Some("sandbox-abc"));

        // Should be retrievable
        let fetched = get_provision(call_id);
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().phase, ProvisionPhase::Ready);

        // Active provisions should be empty (Ready is terminal)
        let active = list_active_provisions();
        assert!(active.is_empty());
    }
}
