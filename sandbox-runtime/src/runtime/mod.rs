use docktopus::DockerBuilder;
use docktopus::bollard::container::{Config as BollardConfig, LogOutput, RemoveContainerOptions};
use docktopus::bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use once_cell::sync::OnceCell;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::OnceCell as AsyncOnceCell;
use tokio_stream::StreamExt;

// ---------------------------------------------------------------------------
// Per-sandbox lifecycle lock
// ---------------------------------------------------------------------------

/// Striped per-sandbox mutex preventing concurrent lifecycle mutations
/// (stop, resume, delete, recreate) on the same sandbox. Without this,
/// concurrent stop+resume or double-inject can create orphaned containers
/// or divergent state.
///
/// Uses DashMap<String, Arc<tokio::sync::Mutex<()>>> so that acquiring a
/// lock for sandbox A does not block operations on sandbox B.
static LIFECYCLE_LOCKS: once_cell::sync::Lazy<
    dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
> = once_cell::sync::Lazy::new(dashmap::DashMap::new);

/// Acquire the per-sandbox lifecycle lock. The returned guard must be held
/// for the entire duration of the lifecycle operation (state check → Docker
/// call → store write). Dropping the guard releases the lock.
pub async fn acquire_lifecycle_lock(sandbox_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let mutex = LIFECYCLE_LOCKS
        .entry(sandbox_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    mutex.lock_owned().await
}

use crate::error::{Result, SandboxError};
use crate::util::{merge_metadata, parse_json_object, shell_escape};
use crate::{DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE, DEFAULT_SIDECAR_SSH_PORT};

// Match the 30s sidecar health-check window for slower CI/coverage runners.
const PORT_MAPPING_RETRY_ATTEMPTS: usize = 60;
const PORT_MAPPING_RETRY_DELAY_MS: u64 = 500;
const SSH_DEFAULT_LOGIN_USER: &str = "sidecar";
const SSH_FALLBACK_LOGIN_USER: &str = "agent";
const SSH_COMPATIBLE_LOGIN_USERS: &[&str] = &[SSH_DEFAULT_LOGIN_USER, SSH_FALLBACK_LOGIN_USER];

mod admission;
mod backend;
mod create;
mod docker_client;
mod docker_config;
mod docker_create;
mod env_vars;
mod firecracker_create;
mod lifecycle;
mod lookup;
mod ports;
mod secrets;
mod snapshots;
mod ssh;
mod ssh_commands;
mod upgrades;

pub(crate) use admission::*;
pub(crate) use backend::*;
pub(crate) use create::*;
pub(crate) use docker_client::*;
pub(crate) use docker_config::*;
pub(crate) use docker_create::*;
pub(crate) use env_vars::*;
pub(crate) use firecracker_create::*;
pub(crate) use lifecycle::*;
pub(crate) use lookup::*;
pub(crate) use ports::*;
#[cfg(test)]
pub(crate) use secrets::*;
pub(crate) use ssh::*;
pub(crate) use ssh_commands::*;

// Externally-reachable items re-exported at their original visibility:
pub use admission::acquire_creation_permit;
pub use create::create_sidecar;
pub use docker_client::docker_builder;
pub use env_vars::{merge_env_json, workflow_runtime_credentials_available};
pub use lifecycle::{
    delete_sidecar, refresh_docker_sandbox_endpoint, resume_sidecar, stop_sidecar,
};
pub use lookup::{
    get_sandbox_by_id, get_sandbox_by_url, get_sandbox_by_url_opt, require_sandbox_owner,
    require_sandbox_owner_by_url, require_sidecar_auth, require_sidecar_owner_auth, touch_sandbox,
};
pub use ports::{PortMapping, PortProtocol, parse_metadata_ports};
pub use secrets::{seal_record, unseal_record};
pub use snapshots::{
    commit_container, create_and_restore_from_s3, create_from_snapshot_image, remove_snapshot_image,
};
pub use ssh::{
    detect_ssh_username, ensure_ssh_ready, provision_ssh_key, restore_ssh_access, revoke_ssh_key,
};
pub use upgrades::{
    SidecarReconcileReport, SidecarUpgradePolicy, current_sidecar_image, reconcile_sidecar_images,
    recreate_sidecar_with_env, sandboxes_needing_image_upgrade, upgrade_sidecar_image,
};

/// ABI-independent parameters for sandbox creation.
///
/// Blueprint-specific job handlers convert their ABI types into this struct
/// before calling `create_sidecar`.
///
/// The sidecar auth token is **always generated server-side** and never
/// included in on-chain calldata. Use the `token_override` parameter on
/// `create_sidecar` when recreating a container that needs to keep its
/// existing token.
#[derive(Clone, Debug, Default)]
pub struct CreateSandboxParams {
    pub name: String,
    pub image: String,
    pub stack: String,
    pub agent_identifier: String,
    pub env_json: String,
    pub metadata_json: String,
    pub ssh_enabled: bool,
    pub ssh_public_key: String,
    /// Deprecated compatibility field: accepted from ABI/config inputs but ignored.
    pub web_terminal_enabled: bool,
    pub max_lifetime_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    /// On-chain caller address (hex string, e.g. "0x1234..."). Set by the job
    /// handler from the `Caller` extractor so that ownership can be enforced.
    pub owner: String,
    /// Service ID that owns the on-chain job used to create this sandbox.
    /// Optional for local-only or legacy sandboxes that were not linked.
    pub service_id: Option<u64>,
    /// Optional TEE configuration. When set with `required: true`, the runtime
    /// must provision the sandbox inside a trusted execution environment.
    pub tee_config: Option<crate::tee::TeeConfig>,
    /// User-injected secrets (phase 2 of two-phase provisioning).
    /// Empty on initial creation; populated when recreating with secrets.
    pub user_env_json: String,
    /// Extra container ports to expose (e.g. user web server on 3000).
    /// Parsed from `metadata_json.ports` at creation time.
    pub port_mappings: Vec<u16>,
    /// Sidecar capabilities to enable at boot, encoded as a JSON array
    /// (e.g. `["computer_use"]`). Currently supported entries:
    /// `computer_use`, `all_harness`.
    /// When non-empty, the runtime sets `SIDECAR_CAPABILITIES` on the
    /// container env so the sidecar boots Xvfb / dbus / MCP at startup.
    /// Empty string means no extra subsystems start.
    pub capabilities_json: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeBackend {
    Docker,
    Firecracker,
    Tee,
}

/// Runtime configuration loaded once at startup from environment variables.
#[derive(Clone, Debug)]
pub struct SidecarRuntimeConfig {
    pub image: String,
    pub public_host: String,
    pub container_port: u16,
    pub ssh_port: u16,
    pub timeout: Duration,
    pub docker_host: Option<String>,
    pub pull_image: bool,
    pub sandbox_default_idle_timeout: u64,
    pub sandbox_default_max_lifetime: u64,
    pub sandbox_max_idle_timeout: u64,
    pub sandbox_max_max_lifetime: u64,
    pub sandbox_reaper_interval: u64,
    pub sandbox_gc_interval: u64,
    pub sandbox_gc_hot_retention: u64,
    pub sandbox_gc_warm_retention: u64,
    pub sandbox_gc_cold_retention: u64,
    pub snapshot_auto_commit: bool,
    pub snapshot_destination_prefix: Option<String>,
    pub sandbox_max_count: usize,
    /// Per-sandbox CPU maximum (cores). 0 = no cap.
    pub sandbox_max_cpu_cores: u64,
    /// Per-sandbox memory maximum (MB). 0 = no cap. Also the value an
    /// unlimited (0) request clamps to, and the footprint an unlimited
    /// sandbox is accounted at in the host memory budget.
    pub sandbox_max_memory_mb: u64,
    /// Per-sandbox disk maximum (GB). 0 = no cap.
    pub sandbox_max_disk_gb: u64,
    /// Total memory (MB) admissible across all running sandboxes. 0 = disabled.
    pub sandbox_host_memory_budget_mb: u64,
    /// Total CPU cores admissible across all running sandboxes. 0 = disabled.
    pub sandbox_host_cpu_budget: u64,
}

static RUNTIME_CONFIG: OnceCell<SidecarRuntimeConfig> = OnceCell::new();

impl SidecarRuntimeConfig {
    /// Compute the effective idle timeout: substitute default for 0, clamp to operator max.
    pub fn effective_idle_timeout(&self, requested: u64) -> u64 {
        let value = if requested == 0 {
            self.sandbox_default_idle_timeout
        } else {
            requested
        };
        value.min(self.sandbox_max_idle_timeout)
    }

    /// Compute the effective max lifetime: substitute default for 0, clamp to operator max.
    pub fn effective_max_lifetime(&self, requested: u64) -> u64 {
        let value = if requested == 0 {
            self.sandbox_default_max_lifetime
        } else {
            requested
        };
        value.min(self.sandbox_max_max_lifetime)
    }

    /// Load configuration from environment variables.
    /// Cached after the first call — subsequent calls return the same config.
    pub fn load() -> &'static SidecarRuntimeConfig {
        RUNTIME_CONFIG.get_or_init(|| {
            let image =
                env::var("SIDECAR_IMAGE").unwrap_or_else(|_| DEFAULT_SIDECAR_IMAGE.to_string());
            let public_host =
                env::var("SIDECAR_PUBLIC_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
            let container_port = env::var("SIDECAR_HTTP_PORT")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(DEFAULT_SIDECAR_HTTP_PORT);
            let ssh_port = env::var("SIDECAR_SSH_PORT")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(DEFAULT_SIDECAR_SSH_PORT);
            let timeout = env::var("REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(crate::DEFAULT_TIMEOUT_SECS);
            let docker_host = env::var("DOCKER_HOST")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(detect_docker_host_fallback);
            let pull_image = env::var("SIDECAR_PULL_IMAGE")
                .ok()
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(true);

            let sandbox_default_idle_timeout = env::var("SANDBOX_DEFAULT_IDLE_TIMEOUT")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1800);
            let sandbox_default_max_lifetime = env::var("SANDBOX_DEFAULT_MAX_LIFETIME")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(86400);
            let sandbox_max_idle_timeout = env::var("SANDBOX_MAX_IDLE_TIMEOUT")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(7200);
            let sandbox_max_max_lifetime = env::var("SANDBOX_MAX_MAX_LIFETIME")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(172800);
            let sandbox_reaper_interval = env::var("SANDBOX_REAPER_INTERVAL")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(30);
            let sandbox_gc_interval = env::var("SANDBOX_GC_INTERVAL")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(3600);
            let sandbox_gc_hot_retention = env::var("SANDBOX_GC_HOT_RETENTION")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .or_else(|| {
                    env::var("SANDBOX_GC_STOPPED_RETENTION")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .unwrap_or(86400);
            let sandbox_gc_warm_retention = env::var("SANDBOX_GC_WARM_RETENTION")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(172800);
            let sandbox_gc_cold_retention = env::var("SANDBOX_GC_COLD_RETENTION")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(604800);
            let snapshot_auto_commit = env::var("SANDBOX_SNAPSHOT_AUTO_COMMIT")
                .ok()
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(true);
            let snapshot_destination_prefix = env::var("SANDBOX_SNAPSHOT_DESTINATION_PREFIX")
                .ok()
                .filter(|v| !v.trim().is_empty());
            let sandbox_max_count = env::var("SANDBOX_MAX_COUNT")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(100);
            let sandbox_max_cpu_cores = env::var("SANDBOX_MAX_CPU_CORES")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let sandbox_max_memory_mb = env::var("SANDBOX_MAX_MEMORY_MB")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let sandbox_max_disk_gb = env::var("SANDBOX_MAX_DISK_GB")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let sandbox_host_memory_budget_mb = env::var("SANDBOX_HOST_MEMORY_BUDGET_MB")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            // Total CPU cores admissible across all running sandboxes. Primary
            // name mirrors SANDBOX_HOST_MEMORY_BUDGET_MB; SANDBOX_CPU_BUDGET is
            // accepted as an alias. 0 = disabled (unlimited).
            let sandbox_host_cpu_budget = env::var("SANDBOX_HOST_CPU_BUDGET")
                .or_else(|_| env::var("SANDBOX_CPU_BUDGET"))
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);

            // Validate critical configuration values. Panics are intentional here —
            // these represent unrecoverable startup misconfigurations. Unlike process::exit,
            // panic! unwinds the stack and runs destructors.
            assert!(!image.trim().is_empty(), "SIDECAR_IMAGE must not be empty");
            assert!(container_port > 0, "SIDECAR_HTTP_PORT must be > 0");
            assert!(timeout > 0, "REQUEST_TIMEOUT_SECS must be > 0");

            tracing::info!(
                image = %image,
                host = %public_host,
                port = container_port,
                idle_timeout = sandbox_default_idle_timeout,
                max_lifetime = sandbox_default_max_lifetime,
                reaper_interval = sandbox_reaper_interval,
                gc_interval = sandbox_gc_interval,
                max_sandboxes = sandbox_max_count,
                max_cpu_cores = sandbox_max_cpu_cores,
                max_memory_mb = sandbox_max_memory_mb,
                max_disk_gb = sandbox_max_disk_gb,
                host_memory_budget_mb = sandbox_host_memory_budget_mb,
                host_cpu_budget = sandbox_host_cpu_budget,
                "Runtime configuration loaded"
            );

            SidecarRuntimeConfig {
                image,
                public_host,
                container_port,
                ssh_port,
                timeout: Duration::from_secs(timeout),
                docker_host,
                pull_image,
                sandbox_default_idle_timeout,
                sandbox_default_max_lifetime,
                sandbox_max_idle_timeout,
                sandbox_max_max_lifetime,
                sandbox_reaper_interval,
                sandbox_gc_interval,
                sandbox_gc_hot_retention,
                sandbox_gc_warm_retention,
                sandbox_gc_cold_retention,
                snapshot_auto_commit,
                snapshot_destination_prefix,
                sandbox_max_count,
                sandbox_max_cpu_cores,
                sandbox_max_memory_mb,
                sandbox_max_disk_gb,
                sandbox_host_memory_budget_mb,
                sandbox_host_cpu_budget,
            }
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SandboxState {
    #[default]
    Running,
    Stopped,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SandboxRecord {
    pub id: String,
    pub container_id: String,
    pub sidecar_url: String,
    pub sidecar_port: u16,
    pub ssh_port: Option<u16>,
    pub token: String,
    pub created_at: u64,
    #[serde(default)]
    pub cpu_cores: u64,
    #[serde(default)]
    pub memory_mb: u64,
    #[serde(default)]
    pub state: SandboxState,
    #[serde(default)]
    pub idle_timeout_seconds: u64,
    #[serde(default)]
    pub max_lifetime_seconds: u64,
    #[serde(default)]
    pub last_activity_at: u64,
    #[serde(default)]
    pub stopped_at: Option<u64>,
    #[serde(default)]
    pub snapshot_image_id: Option<String>,
    #[serde(default)]
    pub snapshot_s3_url: Option<String>,
    #[serde(default)]
    pub container_removed_at: Option<u64>,
    #[serde(default)]
    pub image_removed_at: Option<u64>,
    #[serde(default)]
    pub original_image: String,
    /// Base environment variables set at creation time (immutable).
    #[serde(default, alias = "env_json")]
    pub base_env_json: String,
    /// User-injected secrets via two-phase provisioning (mutable).
    #[serde(default)]
    pub user_env_json: String,
    #[serde(default)]
    pub snapshot_destination: Option<String>,
    /// Backend-specific deployment ID for TEE sandboxes (e.g. Phala app_id).
    #[serde(default)]
    pub tee_deployment_id: Option<String>,
    /// Opaque backend metadata JSON for TEE sandboxes.
    #[serde(default)]
    pub tee_metadata_json: Option<String>,
    /// Deploy-time attestation report serialized as JSON.
    #[serde(default)]
    pub tee_attestation_json: Option<String>,
    // ── Creation params preserved for recreation ──────────────────────────
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub agent_identifier: String,
    #[serde(default)]
    pub metadata_json: String,
    #[serde(default)]
    pub disk_gb: u64,
    #[serde(default)]
    pub stack: String,
    /// On-chain address of the caller who created this sandbox. Used for
    /// ownership checks — only the owner may stop, resume, or delete a sandbox.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub service_id: Option<u64>,
    /// TEE configuration used to create this sandbox (preserved for recreation).
    #[serde(default)]
    pub tee_config: Option<crate::tee::TeeConfig>,
    /// Extra user-requested port mappings: container_port → host_port.
    /// Populated from `metadata_json.ports` at creation time.
    #[serde(default)]
    pub extra_ports: HashMap<u16, u16>,
    /// SSH login user chosen by the runtime when SSH is enabled.
    #[serde(default)]
    pub ssh_login_user: Option<String>,
    /// Persisted SSH key assignments so they can be replayed after recreation.
    #[serde(default)]
    pub ssh_authorized_keys: Vec<SshAuthorizedKey>,
    /// Sidecar capabilities the sandbox was created with (e.g.
    /// `["computer_use"]`), preserved verbatim from the create request
    /// so snapshot-restore and recreation hand the same capability set
    /// back to the sidecar. Empty string when no extra capabilities
    /// were requested.
    #[serde(default)]
    pub capabilities_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SshAuthorizedKey {
    pub username: String,
    pub public_key: String,
}

impl SandboxRecord {
    /// Whether the user has injected secrets via two-phase provisioning.
    pub fn has_user_secrets(&self) -> bool {
        let s = self.user_env_json.trim();
        !s.is_empty() && s != "{}"
    }

    /// Merge base + user env into a single JSON string for container creation.
    pub fn effective_env_json(&self) -> String {
        merge_env_json(&self.base_env_json, &self.user_env_json)
    }
}

use crate::store::PersistentStore;

static SANDBOXES: OnceCell<PersistentStore<SandboxRecord>> = OnceCell::new();
static INSTANCE_STORE: OnceCell<PersistentStore<SandboxRecord>> = OnceCell::new();
static IMAGE_PULLED: AsyncOnceCell<()> = AsyncOnceCell::const_new();

/// Access the fleet-mode sandbox store (`sandboxes.json`), initializing it on first call.
pub fn sandboxes() -> Result<&'static PersistentStore<SandboxRecord>> {
    SANDBOXES
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("sandboxes.json");
            PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

/// Best-effort repair for legacy cloud sandbox records that were persisted
/// without their `service_id`.
///
/// We only backfill when the provision tracker can prove the relationship via
/// `metadata.service_id` for the same `sandbox_id`. If no lineage is present,
/// the record is left unchanged.
pub fn repair_sandbox_service_links_from_provisions() -> Result<usize> {
    let provisions = crate::provision_progress::list_all_provisions()?;
    if provisions.is_empty() {
        return Ok(0);
    }

    let mut service_by_sandbox_id = HashMap::<String, u64>::new();
    for provision in provisions {
        let Some(sandbox_id) = provision.sandbox_id else {
            continue;
        };
        let Some(service_id) = provision
            .metadata
            .get("service_id")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        service_by_sandbox_id
            .entry(sandbox_id)
            .or_insert(service_id);
    }

    if service_by_sandbox_id.is_empty() {
        return Ok(0);
    }

    let store = sandboxes()?;
    let records = store.values()?;
    let mut repaired = 0usize;

    for record in records {
        if record.service_id.is_some() {
            continue;
        }
        let Some(service_id) = service_by_sandbox_id.get(&record.id).copied() else {
            continue;
        };
        if store.update(&record.id, |entry| {
            if entry.service_id.is_none() {
                entry.service_id = Some(service_id);
            }
        })? {
            repaired += 1;
        }
    }

    Ok(repaired)
}

/// Access the instance-mode singleton sandbox store (`instance.json`).
///
/// In instance mode, a single sandbox is stored under key `"instance"`.
/// This is the same file written by `set_instance_sandbox()` in the instance
/// blueprint lib. The operator API reads from it for `/api/sandbox/*` routes.
pub fn instance_store() -> Result<&'static PersistentStore<SandboxRecord>> {
    INSTANCE_STORE
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("instance.json");
            PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

/// Get the instance-mode singleton sandbox, if provisioned.
pub fn get_instance_sandbox() -> Result<Option<SandboxRecord>> {
    match instance_store()?.get("instance")? {
        Some(mut r) => {
            unseal_record(&mut r)?;
            Ok(Some(r))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
