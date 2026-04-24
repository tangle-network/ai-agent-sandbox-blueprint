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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeBackend {
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

/// Build a fresh Docker client for each call.
///
/// We intentionally do not cache the builder for the life of the process so
/// Docker Desktop socket or port-mapping state cannot go stale across long-lived
/// operator sessions.
pub async fn docker_builder() -> Result<DockerBuilder> {
    let config = SidecarRuntimeConfig::load();
    match config.docker_host.as_deref() {
        Some(host) => DockerBuilder::with_address(host).await.map_err(|err| {
            SandboxError::Docker(format!("Failed to connect to Docker at {host}: {err}"))
        }),
        None => DockerBuilder::new()
            .await
            .map_err(|err| SandboxError::Docker(format!("Failed to connect to Docker: {err}"))),
    }
}

fn detect_docker_host_fallback() -> Option<String> {
    let default_socket = std::path::Path::new("/var/run/docker.sock");
    if default_socket.exists() {
        return None;
    }

    let home = env::var("HOME").ok()?;
    let docker_desktop_socket = std::path::Path::new(&home).join(".docker/run/docker.sock");
    docker_desktop_socket
        .exists()
        .then(|| format!("unix://{}", docker_desktop_socket.display()))
}

/// Default timeout for Docker operations (seconds).
const DEFAULT_DOCKER_TIMEOUT_SECS: u64 = 60;

/// Wrap a Docker future in a timeout to prevent indefinite hangs.
///
/// Reads `DOCKER_OPERATION_TIMEOUT_SECS` env var (default: 60s).
pub(crate) async fn docker_timeout<F, T, E>(op_name: &str, future: F) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let timeout_secs = env::var("DOCKER_OPERATION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DOCKER_TIMEOUT_SECS);

    let start = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), future).await;
    let duration_ms = start.elapsed().as_millis();
    tracing::debug!(op = %op_name, duration_ms = %duration_ms, "docker operation completed");

    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(SandboxError::Docker(format!("{op_name} failed: {e}"))),
        Err(_) => Err(SandboxError::Docker(format!(
            "{op_name} timed out after {timeout_secs}s"
        ))),
    }
}

/// Generic retry helper for Docker operations.
///
/// Retries `f` up to `max_retries` times with exponential backoff starting at
/// `backoff_ms`. On each failure (except the last), a warning is logged and the
/// operation is retried after sleeping.
async fn retry_docker<F, Fut, T>(
    op_name: &str,
    max_retries: u32,
    backoff_ms: u64,
    f: F,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = None;
    for attempt in 0..=max_retries {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt < max_retries {
                    tracing::warn!(
                        op = op_name,
                        attempt = attempt + 1,
                        error = %e,
                        "Docker operation failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms * (attempt as u64 + 1)))
                        .await;
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        crate::error::SandboxError::Docker(format!(
            "{op_name}: all retries exhausted with no error"
        ))
    }))
}

/// Start a container with a single retry on transient failure.
///
/// Container starts occasionally fail due to Docker daemon contention or
/// transient resource issues. A single retry with 500ms backoff handles the
/// common case without adding excessive latency.
async fn start_container_with_retry(container: &mut Container) -> Result<()> {
    match docker_timeout("start_container", container.start(false)).await {
        Ok(()) => Ok(()),
        Err(first_err) => {
            tracing::warn!(
                error = %first_err,
                "Container start failed, retrying after 500ms"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            docker_timeout("start_container_retry", container.start(false)).await
        }
    }
}

fn existing_store_entry_for_override(sandbox_id: &str) -> Result<Option<SandboxRecord>> {
    sandboxes()?.get(sandbox_id)
}

fn adjusted_sandbox_count_for_limit(current: usize, reusing_existing_slot: bool) -> usize {
    if reusing_existing_slot {
        current.saturating_sub(1)
    } else {
        current
    }
}

/// Global creation permit — serializes the count-check + container-create
/// sequence to prevent TOCTOU races where N concurrent creates all pass the
/// count limit check and then all succeed, exceeding the configured maximum.
///
/// The permit is held from count check through store insertion. Other
/// lifecycle operations (stop, resume) use the per-sandbox lock and do NOT
/// contend on this.
static CREATION_PERMIT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Acquire the creation permit. Must be held across the count check AND the
/// container creation + store insert sequence.
pub async fn acquire_creation_permit() -> tokio::sync::MutexGuard<'static, ()> {
    CREATION_PERMIT.lock().await
}

fn enforce_sandbox_count_limit(
    config: &SidecarRuntimeConfig,
    reusing_existing_slot: bool,
) -> Result<()> {
    if config.sandbox_max_count == 0 {
        return Ok(());
    }

    let current = sandboxes()?.values()?.len();
    let effective_current = adjusted_sandbox_count_for_limit(current, reusing_existing_slot);
    if effective_current >= config.sandbox_max_count {
        return Err(SandboxError::Validation(format!(
            "Sandbox limit reached ({current}/{max}). Delete unused sandboxes before creating new ones.",
            max = config.sandbox_max_count,
        )));
    }

    Ok(())
}

fn restore_previous_store_entry(
    sandbox_id: &str,
    previous_record: Option<SandboxRecord>,
) -> Result<()> {
    match previous_record {
        Some(record) => sandboxes()?.insert(sandbox_id.to_string(), record),
        None => {
            let _ = sandboxes()?.remove(sandbox_id)?;
            Ok(())
        }
    }
}

fn is_retryable_port_mapping_error(err: &SandboxError) -> bool {
    let SandboxError::Docker(msg) = err else {
        return false;
    };

    msg.starts_with("Missing container port mappings")
        || msg.starts_with("Missing port bindings for ")
        || msg.starts_with("Missing host port for ")
        || (msg.starts_with("Host port for ") && msg.ends_with(" is not assigned yet"))
}

async fn retry_port_mapping_lookup_inner<T, F, Fut>(
    operation: &str,
    container_id: &str,
    max_attempts: usize,
    delay_ms: u64,
    mut f: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = None;
    tracing::info!(
        operation,
        container_id,
        "Resolving published sidecar endpoint"
    );

    for attempt in 0..max_attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !is_retryable_port_mapping_error(&err) {
                    return Err(err);
                }
                last_err = Some(err);
                if attempt + 1 < max_attempts {
                    tracing::warn!(
                        operation,
                        container_id,
                        attempt = attempt + 1,
                        max_attempts,
                        error = %last_err.as_ref().expect("last_err just set"),
                        "Published sidecar endpoint not ready yet, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    let last_err = last_err.unwrap_or_else(|| {
        SandboxError::Unavailable(format!(
            "Unable to resolve published sidecar endpoint for container {container_id}"
        ))
    });
    Err(SandboxError::Unavailable(format!(
        "{operation} failed: Docker did not publish sidecar port for container {container_id} after {max_attempts} attempts: {last_err}"
    )))
}

/// Best-effort removal of an orphaned container after a partial creation failure.
async fn cleanup_orphaned_container(builder: &DockerBuilder, container_id: &str) {
    tracing::warn!(
        container_id,
        "Cleaning up orphaned container after creation failure"
    );
    let timeout = std::time::Duration::from_secs(30);
    let result = tokio::time::timeout(timeout, async {
        if let Ok(c) = Container::from_id(builder.client(), container_id).await {
            let _ = c
                .remove(Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }))
                .await;
        }
    })
    .await;
    if result.is_err() {
        tracing::error!(container_id, "Orphan container cleanup timed out after 30s");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// At-rest encryption for secrets stored in SandboxRecord
// ─────────────────────────────────────────────────────────────────────────────

/// Prefix that marks a field as encrypted (enables transparent migration).
const ENC_PREFIX: &str = "enc:v1:";

/// HKDF info parameter for secrets-at-rest key derivation (distinct from PASETO).
const SECRETS_HKDF_INFO: &[u8] = b"secrets-at-rest-encryption-v1";

/// HKDF salt — shared with session_auth to derive from the same root secret,
/// but the distinct `info` parameter ensures an independent key.
const SECRETS_HKDF_SALT: &[u8] = b"tangle-sandbox-blueprint-paseto-v4";

/// 256-bit encryption key derived from `SESSION_AUTH_SECRET` via HKDF-SHA256.
/// Falls back to an ephemeral random key (with warning) if the env var is unset.
static SEAL_KEY: once_cell::sync::Lazy<[u8; 32]> = once_cell::sync::Lazy::new(|| {
    use hkdf::Hkdf;
    use sha2::Sha256;

    match std::env::var("SESSION_AUTH_SECRET") {
        Ok(secret) => {
            let hk = Hkdf::<Sha256>::new(Some(SECRETS_HKDF_SALT), secret.as_bytes());
            let mut key = [0u8; 32];
            hk.expand(SECRETS_HKDF_INFO, &mut key)
                .expect("HKDF-SHA256 expand to 32 bytes cannot fail");
            key
        }
        Err(_) => {
            tracing::warn!(
                "SESSION_AUTH_SECRET not set; using ephemeral key for secrets encryption. \
                 Stored secrets will NOT survive restart."
            );
            let mut key = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
            key
        }
    }
});

/// Encrypt a plaintext string using ChaCha20-Poly1305 AEAD.
/// Returns `"enc:v1:" + base64(nonce || ciphertext)`.
fn seal_field(plaintext: &str) -> Result<String> {
    use base64::Engine;
    use chacha20poly1305::{
        AeadCore, ChaCha20Poly1305, KeyInit,
        aead::{Aead, OsRng},
    };

    if plaintext.is_empty() {
        return Ok(String::new());
    }

    let cipher = ChaCha20Poly1305::new((&*SEAL_KEY).into());
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| SandboxError::Storage(format!("seal_field encrypt failed: {e}")))?;

    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);

    Ok(format!(
        "{ENC_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    ))
}

/// Decrypt a stored field. If it doesn't carry the `enc:v1:` prefix, return as-is
/// (transparent migration from plaintext).
fn unseal_field(stored: &str) -> Result<String> {
    use base64::Engine;
    use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::Aead};

    if stored.is_empty() {
        return Ok(stored.to_string());
    }
    if !stored.starts_with(ENC_PREFIX) {
        // Migration path: pre-encryption records stored as plaintext.
        // This passthrough will be removed in a future release.
        tracing::warn!(
            "unseal_field: found unencrypted value — records will be re-encrypted on next write"
        );
        return Ok(stored.to_string());
    }

    let encoded = &stored[ENC_PREFIX.len()..];
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| SandboxError::Storage(format!("unseal_field base64 decode failed: {e}")))?;

    if blob.len() < 12 {
        return Err(SandboxError::Storage(
            "unseal_field: ciphertext too short".into(),
        ));
    }

    let nonce = chacha20poly1305::Nonce::from_slice(&blob[..12]);
    let ciphertext = &blob[12..];

    let cipher = ChaCha20Poly1305::new((&*SEAL_KEY).into());
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| SandboxError::Storage(format!("unseal_field decrypt failed: {e}")))?;

    String::from_utf8(plaintext)
        .map_err(|e| SandboxError::Storage(format!("unseal_field utf8 failed: {e}")))
}

/// Encrypt sensitive fields in a `SandboxRecord` before persisting.
///
/// Returns an error if any field fails to encrypt — never falls back to
/// storing plaintext, which would silently expose secrets at rest.
pub fn seal_record(record: &mut SandboxRecord) -> Result<()> {
    record.token =
        seal_field(&record.token).map_err(|e| SandboxError::Storage(format!("seal token: {e}")))?;
    record.base_env_json = seal_field(&record.base_env_json)
        .map_err(|e| SandboxError::Storage(format!("seal base_env_json: {e}")))?;
    record.user_env_json = seal_field(&record.user_env_json)
        .map_err(|e| SandboxError::Storage(format!("seal user_env_json: {e}")))?;
    Ok(())
}

/// Decrypt sensitive fields in a `SandboxRecord` after reading from store.
///
/// Returns an error if any field fails to decrypt. This prevents passing
/// garbled ciphertext to sidecars as credentials or environment variables.
pub fn unseal_record(record: &mut SandboxRecord) -> Result<()> {
    record.token = unseal_field(&record.token)
        .map_err(|e| SandboxError::Storage(format!("unseal token: {e}")))?;
    record.base_env_json = unseal_field(&record.base_env_json)
        .map_err(|e| SandboxError::Storage(format!("unseal base_env_json: {e}")))?;
    record.user_env_json = unseal_field(&record.user_env_json)
        .map_err(|e| SandboxError::Storage(format!("unseal user_env_json: {e}")))?;
    Ok(())
}

fn next_sandbox_id() -> String {
    format!("sandbox-{}", uuid::Uuid::new_v4())
}

pub fn get_sandbox_by_id(id: &str) -> Result<SandboxRecord> {
    let mut record = sandboxes()?
        .get(id)?
        .ok_or_else(|| SandboxError::NotFound(format!("Sandbox '{id}' not found")))?;
    unseal_record(&mut record)?;
    Ok(record)
}

pub fn get_sandbox_by_url(sidecar_url: &str) -> Result<SandboxRecord> {
    let url = sidecar_url.to_string();
    let mut record = sandboxes()?
        .find(|record| record.sidecar_url == url)?
        .ok_or_else(|| {
            SandboxError::NotFound(format!("Sandbox not found for URL: {sidecar_url}"))
        })?;
    unseal_record(&mut record)?;
    Ok(record)
}

/// Update `last_activity_at` to now for the given sandbox.
pub fn touch_sandbox(sandbox_id: &str) {
    if let Ok(store) = sandboxes() {
        let now = crate::util::now_ts();
        let _ = store.update(sandbox_id, |r| {
            r.last_activity_at = now;
        });
    }
}

/// Find a sandbox by its sidecar URL, returning `None` instead of an error if not found.
pub fn get_sandbox_by_url_opt(sidecar_url: &str) -> Option<SandboxRecord> {
    let url = sidecar_url.to_string();
    sandboxes().ok().and_then(|store| {
        store
            .find(|record| record.sidecar_url == url)
            .ok()
            .flatten()
            .and_then(|mut r| unseal_record(&mut r).ok().map(|()| r))
    })
}

/// Validate that `caller` owns the sandbox, returning the record on success.
pub fn require_sandbox_owner(sandbox_id: &str, caller: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_id(sandbox_id)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth(format!(
            "Sandbox '{sandbox_id}' has no owner configured"
        )));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox '{sandbox_id}'"
        )))
    }
}

/// Validate that `caller` owns the sandbox at `sidecar_url` AND the token matches.
pub fn require_sidecar_owner_auth(
    sidecar_url: &str,
    token: &str,
    caller: &str,
) -> Result<SandboxRecord> {
    let record = require_sidecar_auth(sidecar_url, token)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth("Sandbox has no owner configured".into()));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox at '{sidecar_url}'"
        )))
    }
}

/// Validate that `caller` owns the sandbox at `sidecar_url` (no token required).
///
/// Used by job handlers where the on-chain `Caller` extractor provides auth and
/// the sidecar token is looked up from the stored `SandboxRecord`.
pub fn require_sandbox_owner_by_url(sidecar_url: &str, caller: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_url(sidecar_url)?;
    if record.owner.is_empty() {
        return Err(SandboxError::Auth("Sandbox has no owner configured".into()));
    }
    if record.owner.eq_ignore_ascii_case(caller) {
        Ok(record)
    } else {
        Err(SandboxError::Auth(format!(
            "Caller {caller} does not own sandbox at '{sidecar_url}'"
        )))
    }
}

/// Validate sidecar token using constant-time comparison to prevent timing attacks.
pub fn require_sidecar_auth(sidecar_url: &str, token: &str) -> Result<SandboxRecord> {
    let record = get_sandbox_by_url(sidecar_url)?;
    if record.token.as_bytes().ct_eq(token.as_bytes()).into() {
        Ok(record)
    } else {
        Err(SandboxError::Auth("Unauthorized sidecar_token".into()))
    }
}

/// Ensure the sidecar image is available locally. Pulls once on first call
/// if `SIDECAR_PULL_IMAGE` is true. Subsequent calls are no-ops.
///
/// Image pulls are retried up to 2 times with 1-second backoff to handle
/// transient registry errors.
async fn ensure_image_pulled(builder: &DockerBuilder, image: &str) -> Result<()> {
    IMAGE_PULLED
        .get_or_try_init(|| async {
            let config = SidecarRuntimeConfig::load();
            if config.pull_image {
                retry_docker("pull_image", 2, 1000, || {
                    docker_timeout("pull_image", builder.pull_image(image, None))
                })
                .await?;
            }
            Ok::<(), SandboxError>(())
        })
        .await?;
    Ok(())
}

/// Create a new sandbox container.
///
/// `token_override`: when `Some`, uses the given token instead of generating
/// a new one. Used by `recreate_sidecar_with_env` to preserve the original
/// token across container re-creation.
pub async fn create_sidecar(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    create_sidecar_with_token(request, tee, None, None).await
}

/// Internal: create sidecar with optional token override.
///
/// Acquires [`CREATION_PERMIT`] to serialize the count-check + create
/// sequence and prevent TOCTOU races on the sandbox limit.
async fn create_sidecar_with_token(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    let _creation_permit = acquire_creation_permit().await;
    match resolve_runtime_backend(request)? {
        RuntimeBackend::Tee => {
            let backend = tee.ok_or_else(|| {
                SandboxError::Validation(
                    "TEE runtime selected but no TEE backend configured".into(),
                )
            })?;
            validate_requested_tee_backend(request, backend)?;
            create_sidecar_tee(request, backend, token_override, sandbox_id_override).await
        }
        RuntimeBackend::Firecracker => {
            create_sidecar_firecracker(request, token_override, sandbox_id_override)
                .await
                .map(|r| (r, None))
        }
        RuntimeBackend::Docker => {
            create_sidecar_docker(request, token_override, sandbox_id_override)
                .await
                .map(|r| (r, None))
        }
    }
}

fn validate_requested_tee_backend(
    request: &CreateSandboxParams,
    backend: &dyn crate::tee::TeeBackend,
) -> Result<()> {
    let Some(config) = request.tee_config.as_ref() else {
        return Ok(());
    };

    if let Some(nonce) = &config.attestation_nonce {
        crate::tee::validate_attestation_nonce(nonce)?;
        if !nonce.is_empty() && !backend.supports_attestation_report_data() {
            return Err(SandboxError::Validation(format!(
                "TEE backend {:?} does not support caller-supplied attestation nonces",
                backend.tee_type()
            )));
        }
    }

    if config.required
        && config.tee_type != crate::tee::TeeType::None
        && config.tee_type != backend.tee_type()
    {
        return Err(SandboxError::Validation(format!(
            "Requested TEE type {:?} is not available on configured backend {:?}",
            config.tee_type,
            backend.tee_type()
        )));
    }

    Ok(())
}

async fn create_sidecar_tee(
    request: &CreateSandboxParams,
    backend: &dyn crate::tee::TeeBackend,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };

    let extra_ports = parse_extra_ports(&request.metadata_json, &request.port_mappings);
    let mut tee_request = request.clone();
    tee_request.port_mappings = extra_ports;

    let tee_params = crate::tee::TeeDeployParams::from_sandbox_params(
        &sandbox_id,
        &tee_request,
        config.container_port,
        config.ssh_port,
        &token,
    );

    let deployment = backend.deploy(&tee_params).await?;

    let now = crate::util::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: format!("tee-{}", deployment.deployment_id),
        sidecar_url: deployment.sidecar_url,
        sidecar_port: config.container_port,
        ssh_port: deployment.ssh_port,
        token,
        created_at: now,
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
        state: SandboxState::Running,
        idle_timeout_seconds: idle_timeout,
        max_lifetime_seconds: max_lifetime,
        last_activity_at: now,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: request.image.clone(),
        base_env_json: request.env_json.clone(),
        user_env_json: String::new(),
        snapshot_destination: None,
        tee_deployment_id: Some(deployment.deployment_id),
        tee_metadata_json: Some(deployment.metadata_json),
        tee_attestation_json: serde_json::to_string(&deployment.attestation).ok(),
        name: request.name.clone(),
        agent_identifier: request.agent_identifier.clone(),
        metadata_json: request.metadata_json.clone(),
        disk_gb: request.disk_gb,
        stack: request.stack.clone(),
        owner: request.owner.clone(),
        service_id: request.service_id,
        tee_config: request.tee_config.clone(),
        extra_ports: deployment.extra_ports,
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed)?;
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok((record, Some(deployment.attestation)))
}

fn parse_runtime_backend_value(value: &str) -> Option<RuntimeBackend> {
    match value.trim().to_ascii_lowercase().as_str() {
        "docker" | "container" => Some(RuntimeBackend::Docker),
        "firecracker" | "microvm" => Some(RuntimeBackend::Firecracker),
        "tee" | "confidential" | "confidential-vm" => Some(RuntimeBackend::Tee),
        _ => None,
    }
}

fn runtime_backend_name(backend: RuntimeBackend) -> &'static str {
    match backend {
        RuntimeBackend::Docker => "docker",
        RuntimeBackend::Firecracker => "firecracker",
        RuntimeBackend::Tee => "tee",
    }
}

fn metadata_with_runtime_backend(metadata_json: &str, backend: RuntimeBackend) -> Result<String> {
    let mut map = match parse_json_object(metadata_json, "metadata_json")? {
        Some(Value::Object(map)) => map,
        None => Map::new(),
        Some(_) => {
            return Err(SandboxError::Validation(
                "metadata_json must be a JSON object".into(),
            ));
        }
    };

    map.insert(
        "runtime_backend".to_string(),
        Value::String(runtime_backend_name(backend).to_string()),
    );

    serde_json::to_string(&Value::Object(map))
        .map_err(|e| SandboxError::Validation(format!("failed to serialize metadata_json: {e}")))
}

fn parse_runtime_backend_from_metadata(metadata_json: &str) -> Result<Option<RuntimeBackend>> {
    let metadata = parse_json_object(metadata_json, "metadata_json")?;
    let Some(meta) = metadata else {
        return Ok(None);
    };

    let backend = meta
        .get("runtime_backend")
        .and_then(|v| v.as_str())
        .or_else(|| {
            meta.get("runtime")
                .and_then(|v| v.get("backend"))
                .and_then(|v| v.as_str())
        });

    let Some(raw) = backend else {
        return Ok(None);
    };

    parse_runtime_backend_value(raw).map(Some).ok_or_else(|| {
        SandboxError::Validation(format!(
            "metadata_json.runtime_backend must be one of: docker, firecracker, tee (got '{raw}')"
        ))
    })
}

fn parse_runtime_backend_from_env() -> Result<RuntimeBackend> {
    let raw = std::env::var("SANDBOX_RUNTIME_BACKEND").unwrap_or_else(|_| "docker".to_string());
    parse_runtime_backend_value(&raw).ok_or_else(|| {
        SandboxError::Validation(format!(
            "SANDBOX_RUNTIME_BACKEND must be one of: docker, firecracker, tee (got '{raw}')"
        ))
    })
}

fn resolve_runtime_backend(request: &CreateSandboxParams) -> Result<RuntimeBackend> {
    let metadata_backend = parse_runtime_backend_from_metadata(&request.metadata_json)?;
    let selected = match metadata_backend {
        Some(b) => b,
        None => parse_runtime_backend_from_env()?,
    };

    let tee_required = request.tee_config.as_ref().is_some_and(|cfg| cfg.required);
    if tee_required {
        if selected == RuntimeBackend::Firecracker {
            return Err(SandboxError::Validation(
                "runtime_backend=firecracker is incompatible with tee_required=true".into(),
            ));
        }
        return Ok(RuntimeBackend::Tee);
    }

    Ok(selected)
}

fn runtime_backend_for_record(record: &SandboxRecord) -> RuntimeBackend {
    if record.tee_deployment_id.is_some() {
        return RuntimeBackend::Tee;
    }
    match parse_runtime_backend_from_metadata(&record.metadata_json) {
        Ok(Some(backend)) => backend,
        Ok(None) => RuntimeBackend::Docker,
        Err(err) => {
            tracing::warn!(
                sandbox_id = %record.id,
                error = %err,
                "invalid metadata_json.runtime_backend on stored record; defaulting to docker backend"
            );
            RuntimeBackend::Docker
        }
    }
}

pub(crate) fn record_uses_firecracker(record: &SandboxRecord) -> bool {
    runtime_backend_for_record(record) == RuntimeBackend::Firecracker
}

pub fn supports_docker_endpoint_refresh(record: &SandboxRecord) -> bool {
    record.tee_deployment_id.is_none() && !record_uses_firecracker(record)
}

fn parse_url_port(url: &str) -> Option<u16> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.port_or_known_default())
}

#[derive(Debug, Default, Clone)]
struct ExecCommandResult {
    exit_code: i64,
    stdout: String,
    stderr: String,
}

fn exec_result_json(result: &ExecCommandResult) -> Value {
    json!({
        "result": {
            "exitCode": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }
    })
}

fn summarize_exec_failure(result: &ExecCommandResult) -> String {
    result
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .or_else(|| {
            result
                .stdout
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
        })
        .unwrap_or("command failed")
        .to_string()
}

fn parse_sidecar_exec_result(parsed: &Value) -> ExecCommandResult {
    let result = parsed.get("result");
    ExecCommandResult {
        exit_code: result
            .and_then(|r| r.get("exitCode"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        stdout: result
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: result
            .and_then(|r| r.get("stderr"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn extract_detected_ssh_username(result: &ExecCommandResult) -> Result<String> {
    if result.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH username detection failed (exit {}): {}",
            result.exit_code,
            summarize_exec_failure(result)
        )));
    }

    for line in result.stdout.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if crate::ssh_validation::validate_ssh_username(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(SandboxError::Validation(
        "SSH username detection failed: could not find a valid username in command output".into(),
    ))
}

async fn docker_exec_as_user(
    container_id: &str,
    user: &str,
    command: &str,
) -> Result<ExecCommandResult> {
    let builder = docker_builder().await?;
    let exec = docker_timeout(
        "create_exec",
        builder.client().create_exec(
            container_id,
            CreateExecOptions::<String> {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    command.to_string(),
                ]),
                user: Some(user.to_string()),
                ..Default::default()
            },
        ),
    )
    .await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    match docker_timeout(
        "start_exec",
        builder
            .client()
            .start_exec(&exec.id, None::<StartExecOptions>),
    )
    .await?
    {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(chunk) = output.next().await {
                let chunk =
                    chunk.map_err(|e| SandboxError::Docker(format!("exec output failed: {e}")))?;
                match chunk {
                    LogOutput::StdOut { message } | LogOutput::Console { message } => {
                        stdout.extend_from_slice(&message);
                    }
                    LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
                    LogOutput::StdIn { .. } => {}
                }
            }
        }
        StartExecResults::Detached => {
            return Err(SandboxError::Docker(
                "exec unexpectedly detached while waiting for SSH bootstrap output".into(),
            ));
        }
    }

    let inspect = docker_timeout("inspect_exec", builder.client().inspect_exec(&exec.id)).await?;
    Ok(ExecCommandResult {
        exit_code: inspect.exit_code.unwrap_or_default(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn build_docker_ssh_bootstrap_command(username: &str) -> String {
    let user_arg = shell_escape(username);
    format!(
        r#"set -euo pipefail;
user={user_arg};
shell="/bin/sh";
[ -x "$shell" ] || shell="/bin/bash";
if ! getent passwd "$user" >/dev/null 2>&1; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not have a home directory" >&2;
  exit 1;
fi;
current_shell=$(getent passwd "$user" | cut -d: -f7);
if [ "$current_shell" = "/sbin/nologin" ] || [ "$current_shell" = "/bin/false" ]; then
  awk -F: -v user="$user" -v shell="$shell" 'BEGIN {{ OFS=FS }} $1==user {{ $7=shell }} {{ print }}' /etc/passwd > /tmp/passwd.tangle;
  cat /tmp/passwd.tangle > /etc/passwd;
  rm -f /tmp/passwd.tangle;
fi;
if command -v passwd >/dev/null 2>&1; then
  # OpenSSH rejects locked accounts before checking authorized_keys.
  passwd -u "$user" >/dev/null 2>&1 || true;
fi;
if ! command -v sshd >/dev/null 2>&1; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache openssh-server >/dev/null;
  elif command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive;
    apt-get update >/dev/null;
    apt-get install -y --no-install-recommends openssh-server >/dev/null;
    rm -rf /var/lib/apt/lists/*;
  else
    echo "Unsupported package manager for SSH bootstrap" >&2;
    exit 1;
  fi;
fi;
mkdir -p /run/sshd;
ssh-keygen -A >/dev/null 2>&1;
cat > /etc/ssh/sshd_config.tangle <<'EOF'
Port 22
Protocol 2
HostKey /etc/ssh/ssh_host_rsa_key
HostKey /etc/ssh/ssh_host_ed25519_key
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
ChallengeResponseAuthentication no
PermitRootLogin no
AllowUsers {username}
AuthorizedKeysFile .ssh/authorized_keys
PidFile /run/sshd.pid
Subsystem sftp internal-sftp
EOF
if ! awk 'NR > 1 {{ split($2,a,":"); if (toupper(a[2]) == "0016" && $4 == "0A") found=1 }} END {{ exit(found ? 0 : 1) }}' /proc/net/tcp /proc/net/tcp6 2>/dev/null; then
  if [ -f /run/sshd.pid ] && kill -0 "$(cat /run/sshd.pid)" 2>/dev/null; then
    kill "$(cat /run/sshd.pid)" 2>/dev/null || true;
    sleep 1;
  fi;
  rm -f /run/sshd.pid;
  /usr/sbin/sshd -f /etc/ssh/sshd_config.tangle;
fi;
awk 'NR > 1 {{ split($2,a,":"); if (toupper(a[2]) == "0016" && $4 == "0A") found=1 }} END {{ exit(found ? 0 : 1) }}' /proc/net/tcp /proc/net/tcp6 2>/dev/null"#,
    )
}

fn build_docker_ssh_user_home_bootstrap_command(username: &str) -> String {
    let user_arg = shell_escape(username);
    format!(
        r#"set -euo pipefail;
user={user_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
mkdir -p "$home/.ssh";
touch "$home/.ssh/authorized_keys";
chmod 700 "$home/.ssh";
chmod 600 "$home/.ssh/authorized_keys""#
    )
}

fn build_ssh_key_install_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        r#"set -euo pipefail;
user={user_arg};
key={key_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
mkdir -p "$home/.ssh";
touch "$home/.ssh/authorized_keys";
chmod 700 "$home/.ssh";
if ! grep -qxF "$key" "$home/.ssh/authorized_keys" 2>/dev/null; then
  printf '%s\n' "$key" >> "$home/.ssh/authorized_keys";
fi;
chmod 600 "$home/.ssh/authorized_keys""#
    )
}

fn build_ssh_key_revoke_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        r#"set -euo pipefail;
user={user_arg};
key={key_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
if [ -f "$home/.ssh/authorized_keys" ]; then
  tmp=$(mktemp /tmp/authorized_keys.XXXXXX);
  grep -vxF "$key" "$home/.ssh/authorized_keys" > "$tmp" || true;
  mv "$tmp" "$home/.ssh/authorized_keys";
  chmod 600 "$home/.ssh/authorized_keys";
fi"#
    )
}

fn build_sidecar_ssh_key_install_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
if ! grep -qxF {key_arg} \"$home/.ssh/authorized_keys\" 2>/dev/null; then \
    echo {key_arg} >> \"$home/.ssh/authorized_keys\"; \
fi; chmod 600 \"$home/.ssh/authorized_keys\""
    )
}

fn build_sidecar_ssh_key_revoke_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        "set -euo pipefail; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
    tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
    grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
    mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    )
}

fn normalize_requested_ssh_username(username: Option<&str>) -> Result<Option<String>> {
    let Some(username) = username.map(str::trim) else {
        return Ok(None);
    };
    if username.is_empty() {
        return Ok(None);
    }
    crate::ssh_validation::validate_ssh_username(username).map_err(SandboxError::Validation)?;
    Ok(Some(username.to_string()))
}

fn persist_ssh_login_user(sandbox_id: &str, username: &str) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        record.ssh_login_user = Some(username.to_string());
    })?;
    Ok(())
}

fn persist_ssh_key_assignment(sandbox_id: &str, username: &str, public_key: &str) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        let entry = SshAuthorizedKey {
            username: username.to_string(),
            public_key: public_key.to_string(),
        };
        if !record.ssh_authorized_keys.contains(&entry) {
            record.ssh_authorized_keys.push(entry);
        }
    })?;
    Ok(())
}

fn remove_ssh_key_assignment(sandbox_id: &str, username: &str, public_key: &str) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        record
            .ssh_authorized_keys
            .retain(|entry| !(entry.username == username && entry.public_key == public_key));
    })?;
    Ok(())
}

#[cfg(test)]
fn select_docker_ssh_login_user<'a, F>(mut user_exists: F) -> Option<&'a str>
where
    F: FnMut(&str) -> bool,
{
    SSH_COMPATIBLE_LOGIN_USERS
        .iter()
        .copied()
        .find(|candidate| user_exists(candidate))
}

fn compatible_docker_ssh_users_summary() -> String {
    SSH_COMPATIBLE_LOGIN_USERS.join(", ")
}

async fn docker_user_exists(container_id: &str, username: &str) -> Result<bool> {
    let user_arg = shell_escape(username);
    let command = format!("getent passwd {user_arg} >/dev/null 2>&1");
    let result = docker_exec_as_user(container_id, "root", &command).await?;
    Ok(result.exit_code == 0)
}

async fn detect_docker_ssh_username(record: &SandboxRecord) -> Result<String> {
    if let Some(username) = &record.ssh_login_user {
        return Ok(username.clone());
    }

    for candidate in SSH_COMPATIBLE_LOGIN_USERS {
        if docker_user_exists(&record.container_id, candidate).await? {
            persist_ssh_login_user(&record.id, candidate)?;
            return Ok((*candidate).to_string());
        }
    }

    Err(SandboxError::Validation(format!(
        "SSH login user detection failed for sandbox {}: none of the supported users exist (checked: {})",
        record.id,
        compatible_docker_ssh_users_summary()
    )))
}

fn resolve_docker_ssh_username(
    record: &SandboxRecord,
    requested: Option<String>,
) -> Result<String> {
    let login_user = record
        .ssh_login_user
        .clone()
        .unwrap_or_else(|| SSH_DEFAULT_LOGIN_USER.to_string());
    match requested {
        Some(username) if username != login_user => Err(SandboxError::Validation(format!(
            "SSH login is only supported for user '{login_user}'"
        ))),
        Some(username) => Ok(username),
        None => Ok(login_user),
    }
}

async fn ensure_docker_ssh_ready(record: &SandboxRecord) -> Result<String> {
    let login_user = detect_docker_ssh_username(record).await?;
    let root_bootstrap = docker_exec_as_user(
        &record.container_id,
        "root",
        &build_docker_ssh_bootstrap_command(&login_user),
    )
    .await?;
    if root_bootstrap.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH bootstrap failed for sandbox {}: {}",
            record.id,
            summarize_exec_failure(&root_bootstrap)
        )));
    }

    let home_bootstrap = docker_exec_as_user(
        &record.container_id,
        &login_user,
        &build_docker_ssh_user_home_bootstrap_command(&login_user),
    )
    .await?;
    if home_bootstrap.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH bootstrap failed for sandbox {}: {}",
            record.id,
            summarize_exec_failure(&home_bootstrap)
        )));
    }

    persist_ssh_login_user(&record.id, &login_user)?;
    Ok(login_user)
}

fn is_docker_unavailable(err: &SandboxError) -> bool {
    matches!(err, SandboxError::Docker(msg) if msg.contains("Failed to connect to Docker") || msg.contains("Socket not found"))
}

async fn detect_sidecar_ssh_username(record: &SandboxRecord) -> Result<String> {
    let payload = json!({ "command": "id -un || whoami" });
    let parsed = crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await?;
    let username = extract_detected_ssh_username(&parse_sidecar_exec_result(&parsed))?;
    persist_ssh_login_user(&record.id, &username)?;
    Ok(username)
}

async fn execute_docker_ssh_command(
    record: &SandboxRecord,
    user: &str,
    command: &str,
) -> Result<ExecCommandResult> {
    let result = docker_exec_as_user(&record.container_id, user, command).await?;
    if result.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH command failed for sandbox {} (user {}): {}",
            record.id,
            user,
            summarize_exec_failure(&result)
        )));
    }
    Ok(result)
}

async fn execute_sidecar_ssh_command(record: &SandboxRecord, command: &str) -> Result<Value> {
    let payload = json!({ "command": format!("sh -c {}", shell_escape(command)) });
    crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await
}

async fn prepare_ssh_access(record: &SandboxRecord) -> Result<(SandboxRecord, bool)> {
    if record.ssh_port.is_none() {
        return Err(SandboxError::Validation(
            "SSH is not enabled for this sandbox".into(),
        ));
    }

    if supports_docker_endpoint_refresh(record) {
        match ensure_docker_ssh_ready(record).await {
            Ok(_) => return Ok((get_sandbox_by_id(&record.id)?, true)),
            Err(err) if is_docker_unavailable(&err) => {
                return Ok((get_sandbox_by_id(&record.id)?, false));
            }
            Err(err) => return Err(err),
        }
    }

    Ok((get_sandbox_by_id(&record.id)?, false))
}

pub async fn ensure_ssh_ready(record: &SandboxRecord) -> Result<SandboxRecord> {
    let (record, _) = prepare_ssh_access(record).await?;
    Ok(record)
}

pub async fn detect_ssh_username(record: &SandboxRecord) -> Result<String> {
    let (record, docker_managed) = prepare_ssh_access(record).await?;
    if docker_managed {
        return Ok(record
            .ssh_login_user
            .unwrap_or_else(|| SSH_DEFAULT_LOGIN_USER.to_string()));
    }
    if let Some(username) = &record.ssh_login_user {
        return Ok(username.clone());
    }
    detect_sidecar_ssh_username(&record).await
}

pub async fn provision_ssh_key(
    record: &SandboxRecord,
    requested_username: Option<&str>,
    public_key: &str,
) -> Result<(String, Value)> {
    crate::ssh_validation::validate_ssh_public_key(public_key).map_err(SandboxError::Validation)?;
    let requested = normalize_requested_ssh_username(requested_username)?;
    let (ready_record, docker_managed) = prepare_ssh_access(record).await?;
    let username = if docker_managed {
        resolve_docker_ssh_username(&ready_record, requested)?
    } else {
        match requested {
            Some(username) => username,
            None => detect_ssh_username(&ready_record).await?,
        }
    };

    let result_json = if docker_managed {
        exec_result_json(
            &execute_docker_ssh_command(
                &ready_record,
                &username,
                &build_ssh_key_install_command(&username, public_key),
            )
            .await?,
        )
    } else {
        let parsed = execute_sidecar_ssh_command(
            &ready_record,
            &build_sidecar_ssh_key_install_command(&username, public_key),
        )
        .await?;
        let exec = parse_sidecar_exec_result(&parsed);
        if exec.exit_code != 0 {
            return Err(SandboxError::Validation(format!(
                "SSH provision failed for user '{username}' (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(&exec)
            )));
        }
        parsed
    };

    persist_ssh_login_user(&ready_record.id, &username)?;
    persist_ssh_key_assignment(&ready_record.id, &username, public_key)?;
    Ok((username, result_json))
}

pub async fn revoke_ssh_key(
    record: &SandboxRecord,
    requested_username: Option<&str>,
    public_key: &str,
) -> Result<(String, Value)> {
    crate::ssh_validation::validate_ssh_public_key(public_key).map_err(SandboxError::Validation)?;
    let requested = normalize_requested_ssh_username(requested_username)?;
    let (ready_record, docker_managed) = prepare_ssh_access(record).await?;
    let username = if docker_managed {
        resolve_docker_ssh_username(&ready_record, requested)?
    } else {
        match requested {
            Some(username) => username,
            None => detect_ssh_username(&ready_record).await?,
        }
    };

    let result_json = if docker_managed {
        exec_result_json(
            &execute_docker_ssh_command(
                &ready_record,
                &username,
                &build_ssh_key_revoke_command(&username, public_key),
            )
            .await?,
        )
    } else {
        let parsed = execute_sidecar_ssh_command(
            &ready_record,
            &build_sidecar_ssh_key_revoke_command(&username, public_key),
        )
        .await?;
        let exec = parse_sidecar_exec_result(&parsed);
        if exec.exit_code != 0 {
            return Err(SandboxError::Validation(format!(
                "SSH revoke failed for user '{username}' (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(&exec)
            )));
        }
        parsed
    };

    persist_ssh_login_user(&ready_record.id, &username)?;
    remove_ssh_key_assignment(&ready_record.id, &username, public_key)?;
    Ok((username, result_json))
}

pub async fn restore_ssh_access(record: &SandboxRecord) -> Result<SandboxRecord> {
    let (updated, docker_managed) = prepare_ssh_access(record).await?;
    if docker_managed {
        for entry in updated.ssh_authorized_keys.clone() {
            let _ = execute_docker_ssh_command(
                &updated,
                &entry.username,
                &build_ssh_key_install_command(&entry.username, &entry.public_key),
            )
            .await?;
        }
    }
    get_sandbox_by_id(&record.id)
}

async fn create_sidecar_firecracker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let previous_store_entry = existing_store_entry_for_override(&sandbox_id)?;

    enforce_sandbox_count_limit(config, previous_store_entry.is_some())?;

    let extra_ports = parse_extra_ports(&request.metadata_json, &request.port_mappings);
    if !extra_ports.is_empty() {
        return Err(SandboxError::Validation(
            "runtime_backend=firecracker currently does not support metadata_json.ports port mappings"
                .into(),
        ));
    }

    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    let metadata_raw = parse_json_object(&request.metadata_json, "metadata_json")?;
    let snapshot_destination = metadata_raw
        .as_ref()
        .and_then(|v| v.get("snapshot_destination"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let metadata = merge_metadata(metadata_raw, &request.image, &request.stack)?;
    let labels = match metadata {
        Some(Value::Object(map)) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
            .collect::<HashMap<String, String>>(),
        _ => HashMap::new(),
    };

    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let mut env = HashMap::new();
    env.insert(
        "SIDECAR_PORT".to_string(),
        config.container_port.to_string(),
    );
    if !effective_env.trim().is_empty() {
        if let Some(Value::Object(map)) = parse_json_object(&effective_env, "env_json")? {
            for (key, value) in map {
                let val = match value {
                    Value::String(v) => v,
                    Value::Number(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    _ => continue,
                };
                env.insert(key, val);
            }
        }
    }

    let create_request = crate::firecracker::FirecrackerCreateRequest {
        session_id: sandbox_id.clone(),
        image: effective_image.clone(),
        env,
        labels,
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
        disk_gb: request.disk_gb,
    };

    let provisioned = crate::firecracker::create_and_start(create_request).await?;
    let sidecar_url = provisioned.container.endpoint.ok_or_else(|| {
        SandboxError::Unavailable(format!(
            "firecracker host-agent started sandbox {sandbox_id}, but did not return an endpoint"
        ))
    })?;

    let generated_token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };
    let token = provisioned.sidecar_auth_token.unwrap_or(generated_token);
    let metadata_json =
        metadata_with_runtime_backend(&request.metadata_json, RuntimeBackend::Firecracker)?;
    let sidecar_port = parse_url_port(&sidecar_url).unwrap_or(config.container_port);

    let now = crate::util::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id: provisioned.container.id,
        sidecar_url,
        sidecar_port,
        ssh_port: None,
        token,
        created_at: now,
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
        state: SandboxState::Running,
        idle_timeout_seconds: idle_timeout,
        max_lifetime_seconds: max_lifetime,
        last_activity_at: now,
        stopped_at: None,
        snapshot_image_id: None,
        snapshot_s3_url: None,
        container_removed_at: None,
        image_removed_at: None,
        original_image: effective_image,
        base_env_json: request.env_json.clone(),
        user_env_json: request.user_env_json.clone(),
        snapshot_destination,
        tee_deployment_id: None,
        tee_metadata_json: None,
        tee_attestation_json: None,
        name: request.name.clone(),
        agent_identifier: request.agent_identifier.clone(),
        metadata_json,
        disk_gb: request.disk_gb,
        stack: request.stack.clone(),
        owner: request.owner.clone(),
        service_id: request.service_id,
        tee_config: None,
        extra_ports: HashMap::new(),
        ssh_login_user: None,
        ssh_authorized_keys: Vec::new(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed)?;
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok(record)
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared Docker helpers — used by create, snapshot-resume, and S3-restore paths
// ─────────────────────────────────────────────────────────────────────────────

/// Merge base and user env JSON strings into a single JSON object string.
/// User values override base values when keys collide.
pub fn merge_env_json(base: &str, user: &str) -> String {
    let user_trimmed = user.trim();
    if user_trimmed.is_empty() || user_trimmed == "{}" {
        return base.to_string();
    }
    let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(base)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "Failed to parse base_env_json, using empty map");
            serde_json::Map::new()
        });
    if let Ok(user_map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(user) {
        map.extend(user_map);
    }
    serde_json::to_string(&map).unwrap_or_else(|e| {
        tracing::error!(error = %e, "Failed to serialize merged env JSON, returning empty");
        "{}".to_string()
    })
}

pub fn workflow_runtime_credentials_available(env_json: &str) -> Result<bool> {
    let env_map = parse_json_object(env_json, "env_json")?;
    let Some(Value::Object(map)) = env_map else {
        return Ok(false);
    };

    let has_native_provider_key = map
        .get("ANTHROPIC_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || map
            .get("ZAI_API_KEY")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());

    let has_explicit_opencode = map
        .get("OPENCODE_MODEL_PROVIDER")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        && map
            .get("OPENCODE_MODEL_NAME")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && map
            .get("OPENCODE_MODEL_API_KEY")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());

    Ok(has_native_provider_key || has_explicit_opencode)
}

/// Build the `Vec<String>` of `KEY=VALUE` env vars for a Docker container.
fn build_env_vars(env_json: &str, token: &str, container_port: u16) -> Result<Vec<String>> {
    let mut env_vars = vec![
        format!("SIDECAR_PORT={container_port}"),
        format!("SIDECAR_AUTH_TOKEN={token}"),
        // Switch sidecar to container mode so it uses /home/agent (where the
        // Dockerfile pre-creates .local, .cache, .config owned by agent) instead
        // of per-request /tmp/agent/workspace/req-* dirs on tmpfs.
        "AGENT_WORKSPACE_ROOT=/home/agent".to_string(),
        "AGENT_SUBPROCESS_UID=1000".to_string(),
        "AGENT_SUBPROCESS_GID=1000".to_string(),
    ];

    // User-supplied env vars are appended after defaults so they can override.
    let env_map = parse_json_object(env_json, "env_json")?;
    if let Some(Value::Object(map)) = env_map.as_ref() {
        for (key, value) in map {
            let val = match value {
                Value::String(v) => v.clone(),
                Value::Number(v) => v.to_string(),
                Value::Bool(v) => v.to_string(),
                _ => continue,
            };
            env_vars.push(format!("{key}={val}"));
        }
    }
    Ok(env_vars)
}

/// Build the Docker container config override with port bindings, exposed ports,
/// and resource constraints (CPU, memory).
fn build_docker_config(
    config: &SidecarRuntimeConfig,
    ssh_enabled: bool,
    cpu_cores: u64,
    memory_mb: u64,
    labels: Option<HashMap<String, String>>,
    extra_ports: &[u16],
) -> BollardConfig<String> {
    // Security: ports bound to 127.0.0.1 only — not exposed to external network.
    // Inter-container isolation requires Docker daemon --icc=false configuration.
    let mut port_bindings = PortMap::new();
    port_bindings.insert(
        format!("{}/tcp", config.container_port),
        Some(vec![PortBinding {
            host_ip: Some("127.0.0.1".to_string()),
            host_port: None,
        }]),
    );
    if ssh_enabled {
        port_bindings.insert(
            format!("{}/tcp", config.ssh_port),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: None,
            }]),
        );
    }
    for &port in extra_ports {
        port_bindings.insert(
            format!("{port}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: None,
            }]),
        );
    }

    let mut exposed_ports = HashMap::new();
    exposed_ports.insert(format!("{}/tcp", config.container_port), HashMap::new());
    if ssh_enabled {
        exposed_ports.insert(format!("{}/tcp", config.ssh_port), HashMap::new());
    }
    for &port in extra_ports {
        exposed_ports.insert(format!("{port}/tcp"), HashMap::new());
    }

    // When SIDECAR_NETWORK_HOST=true, use host networking so containers share the
    // host's network namespace. This avoids firewall issues where the host drops
    // traffic from the docker bridge interface. Port bindings are ignored in host
    // network mode — the sidecar binds directly on host ports.
    let use_host_network =
        std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");

    let mut host_config = HostConfig {
        port_bindings: if use_host_network {
            None
        } else {
            Some(port_bindings)
        },
        network_mode: if use_host_network {
            Some("host".to_string())
        } else {
            None
        },
        cap_drop: Some(vec!["ALL".to_string()]),
        cap_add: Some({
            let mut caps = vec![
                "SYS_PTRACE".to_string(),
                "SETGID".to_string(),
                "SETUID".to_string(),
                // Agent frameworks (e.g. opencode) chown workspace dirs on startup.
                "CHOWN".to_string(),
            ];
            if ssh_enabled {
                // OpenSSH's pre-auth sandbox chroots into /var/empty.
                caps.push("SYS_CHROOT".to_string());
                caps.push("NET_BIND_SERVICE".to_string());
            }
            caps
        }),
        security_opt: Some(vec!["no-new-privileges=false".to_string()]),
        pids_limit: Some(512),
        readonly_rootfs: Some(false),
        tmpfs: Some(HashMap::from([
            ("/tmp".to_string(), "rw,noexec,nosuid,size=512m".to_string()),
            ("/run".to_string(), "rw,noexec,nosuid,size=64m".to_string()),
        ])),
        // Map host.docker.internal to the host machine so containers can
        // reach host-bound services on the Docker host.
        extra_hosts: if use_host_network {
            None
        } else {
            Some(vec!["host.docker.internal:host-gateway".to_string()])
        },
        ..Default::default()
    };
    if cpu_cores > 0 {
        host_config.nano_cpus = Some((cpu_cores as i64) * 1_000_000_000);
    }
    if memory_mb > 0 {
        host_config.memory = Some((memory_mb as i64) * 1024 * 1024);
    }

    BollardConfig {
        exposed_ports: if use_host_network {
            None
        } else {
            Some(exposed_ports)
        },
        host_config: Some(host_config),
        labels,
        ..Default::default()
    }
}

async fn create_sidecar_docker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
    let previous_store_entry = existing_store_entry_for_override(&sandbox_id)?;

    // Recreating an existing sandbox reuses its existing store slot.
    enforce_sandbox_count_limit(config, previous_store_entry.is_some())?;

    let builder = docker_builder().await?;

    // Use the user-supplied image if provided, otherwise fall back to the
    // operator's SIDECAR_IMAGE env var.
    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    ensure_image_pulled(&builder, &effective_image).await?;
    let original_image = effective_image.clone();

    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };
    let container_name = format!("sidecar-{sandbox_id}");

    let effective_env = merge_env_json(&request.env_json, &request.user_env_json);
    let env_vars = build_env_vars(&effective_env, &token, config.container_port)?;

    let metadata = parse_json_object(&request.metadata_json, "metadata_json")?;
    // Extract snapshot_destination before metadata is consumed by merge/labels
    let snapshot_destination = metadata
        .as_ref()
        .and_then(|v| v.get("snapshot_destination"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let metadata = merge_metadata(metadata, &request.image, &request.stack)?;
    let labels = match metadata {
        Some(Value::Object(map)) => Some(
            map.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
                .collect(),
        ),
        _ => None,
    };

    // Parse extra ports from metadata_json (e.g. {"ports": [3000, 8080]}).
    let extra_ports = parse_extra_ports(&request.metadata_json, &request.port_mappings);

    let override_config = build_docker_config(
        config,
        request.ssh_enabled,
        request.cpu_cores,
        request.memory_mb,
        labels,
        &extra_ports,
    );

    let mut container = Container::new(builder.client(), effective_image)
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let extra_port_seed = extra_ports
            .iter()
            .copied()
            .map(|port| (port, 0u16))
            .collect::<HashMap<_, _>>();
        let (sidecar_url, sidecar_port, ssh_port, extra_port_map) =
            retry_port_mapping_lookup_inner(
                "create endpoint resolution",
                &container_id,
                PORT_MAPPING_RETRY_ATTEMPTS,
                PORT_MAPPING_RETRY_DELAY_MS,
                || {
                    refresh_port_mapping(
                        builder.client(),
                        &container_id,
                        config.container_port,
                        request.ssh_enabled,
                        &config.public_host,
                        &extra_port_seed,
                    )
                },
            )
            .await?;

        // Repair workspace ownership before the sidecar spawns OpenCode as the
        // agent user (uid 1000).  Without this, /home/agent dirs may be root-owned
        // and the demoted process crashes with EACCES on mkdir .local.
        match docker_exec_as_user(
            &container_id,
            "root",
            "chown -R agent:agent /home/agent 2>/dev/null || true",
        )
        .await
        {
            Ok(r) if r.exit_code != 0 => {
                tracing::warn!(
                    sandbox_id,
                    exit_code = r.exit_code,
                    stderr = %r.stderr,
                    "workspace ownership repair returned non-zero (continuing)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    sandbox_id,
                    error = %e,
                    "workspace ownership repair failed (continuing)"
                );
            }
            _ => {}
        }

        // Pre-create directories that the sidecar's root process will try to
        // mkdir before demoting to uid 1000.  Without DAC_OVERRIDE the root
        // process cannot write to agent-owned /home/agent, so we create them
        // as the agent user who legitimately owns the parent directory.
        match docker_exec_as_user(
            &container_id,
            "agent",
            "mkdir -p /home/agent/.opencode-home/.config",
        )
        .await
        {
            Ok(r) if r.exit_code != 0 => {
                tracing::warn!(
                    sandbox_id,
                    exit_code = r.exit_code,
                    stderr = %r.stderr,
                    "opencode-home pre-creation returned non-zero (continuing)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    sandbox_id,
                    error = %e,
                    "opencode-home pre-creation failed (continuing)"
                );
            }
            _ => {}
        }

        let now = crate::util::now_ts();
        let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
        let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

        let record = SandboxRecord {
            id: sandbox_id.clone(),
            container_id: container_id.clone(),
            sidecar_url,
            sidecar_port,
            ssh_port,
            token,
            created_at: now,
            cpu_cores: request.cpu_cores,
            memory_mb: request.memory_mb,
            state: SandboxState::Running,
            idle_timeout_seconds: idle_timeout,
            max_lifetime_seconds: max_lifetime,
            last_activity_at: now,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image,
            base_env_json: request.env_json.clone(),
            user_env_json: request.user_env_json.clone(),
            snapshot_destination,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: request.name.clone(),
            agent_identifier: request.agent_identifier.clone(),
            metadata_json: request.metadata_json.clone(),
            disk_gb: request.disk_gb,
            stack: request.stack.clone(),
            owner: request.owner.clone(),
            service_id: request.service_id,
            tee_config: None,
            extra_ports: extra_port_map,
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
        };

        let mut sealed = record.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(sandbox_id.clone(), sealed)?;

        let ready_record = if request.ssh_enabled {
            ensure_ssh_ready(&record).await?
        } else {
            record.clone()
        };

        crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

        Ok(ready_record)
    }
    .await;

    if finish.is_err() {
        let _ = restore_previous_store_entry(&sandbox_id, previous_store_entry);
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}

/// Stop a running sandbox container, updating its state to `Stopped`.
///
/// For TEE-managed sandboxes, delegates to the TEE backend's `stop()` method.
/// For standard Docker sandboxes, stops via the Docker API directly.
pub async fn stop_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Stopped {
        return Err(SandboxError::Validation(
            "Sandbox is already stopped".into(),
        ));
    }

    // TEE-managed sandbox: delegate to the TEE backend.
    if let Some(deployment_id) = &record.tee_deployment_id {
        if let Some(backend) = crate::tee::try_tee_backend() {
            backend.stop(deployment_id).await?;
            let now = crate::util::now_ts();
            let _ = sandboxes()?.update(&record.id, |r| {
                r.state = SandboxState::Stopped;
                r.stopped_at = Some(now);
            });
            return Ok(());
        }
    }

    if record_uses_firecracker(record) {
        crate::firecracker::stop(&record.container_id).await?;
        let now = crate::util::now_ts();
        let _ = sandboxes()?.update(&record.id, |r| {
            r.state = SandboxState::Stopped;
            r.stopped_at = Some(now);
        });
        return Ok(());
    }

    // Standard Docker path.
    let builder = docker_builder().await?;
    let mut container = docker_timeout(
        "load_container",
        Container::from_id(builder.client(), &record.container_id),
    )
    .await?;
    docker_timeout("stop_container", container.stop()).await?;

    let now = crate::util::now_ts();
    let _ = sandboxes()?.update(&record.id, |r| {
        r.state = SandboxState::Stopped;
        r.stopped_at = Some(now);
    });
    Ok(())
}

/// Poll a sidecar's `/health` endpoint until it responds successfully or the timeout expires.
async fn wait_for_sidecar_health(sidecar_url: &str, timeout_secs: u64) -> bool {
    let ready = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        loop {
            let url = format!("{sidecar_url}/health");
            if let Ok(resp) = crate::util::http_client().map(|c| c.get(&url)) {
                if let Ok(r) = resp.send().await {
                    if r.status().is_success() {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;
    ready.is_ok()
}

async fn refresh_port_mapping_with_retry(
    operation: &str,
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
    container_port: u16,
    ssh_enabled: bool,
    public_host: &str,
    prev_extra_ports: &HashMap<u16, u16>,
) -> Result<(String, u16, Option<u16>, HashMap<u16, u16>)> {
    retry_port_mapping_lookup_inner(
        operation,
        container_id,
        PORT_MAPPING_RETRY_ATTEMPTS,
        PORT_MAPPING_RETRY_DELAY_MS,
        || {
            refresh_port_mapping(
                client.clone(),
                container_id,
                container_port,
                ssh_enabled,
                public_host,
                prev_extra_ports,
            )
        },
    )
    .await
}

/// Re-inspect a running Docker-backed sandbox and persist its current host port mappings.
///
/// This is the authoritative recovery path for stale localhost port bindings
/// after Docker restart/start operations.
pub async fn refresh_docker_sandbox_endpoint(record: &SandboxRecord) -> Result<SandboxRecord> {
    if !supports_docker_endpoint_refresh(record) {
        return Err(SandboxError::Validation(format!(
            "Sandbox {} does not use Docker-backed dynamic port refresh",
            record.id
        )));
    }

    let builder = docker_builder().await?;
    let config = SidecarRuntimeConfig::load();
    let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
        "refresh endpoint resolution",
        builder.client(),
        &record.container_id,
        config.container_port,
        record.ssh_port.is_some(),
        &config.public_host,
        &record.extra_ports,
    )
    .await?;

    let updated = sandboxes()?.update(&record.id, |r| {
        r.sidecar_url = sidecar_url.clone();
        r.sidecar_port = sidecar_port;
        r.ssh_port = ssh_port;
        r.extra_ports = extra_ports.clone();
    })?;

    if !updated {
        return Err(SandboxError::NotFound(format!(
            "Sandbox '{}' not found while refreshing endpoint",
            record.id
        )));
    }

    get_sandbox_by_id(&record.id)
}

async fn stop_started_container(
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
) -> Result<()> {
    let mut container =
        docker_timeout("load_container", Container::from_id(client, container_id)).await?;
    docker_timeout("stop_container", container.stop()).await?;
    Ok(())
}

/// Resume a stopped sandbox, restoring from container, snapshot image, or S3 as available.
pub async fn resume_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Running {
        return Err(SandboxError::Validation(
            "Sandbox is already running".into(),
        ));
    }
    if record_uses_firecracker(record) {
        let resumed = crate::firecracker::start(&record.container_id).await?;
        let sidecar_url = resumed.endpoint.ok_or_else(|| {
            SandboxError::Unavailable(format!(
                "firecracker sandbox {} resumed without sidecar endpoint",
                record.id
            ))
        })?;
        let sidecar_port =
            parse_url_port(&sidecar_url).unwrap_or(SidecarRuntimeConfig::load().container_port);
        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            let _ = crate::firecracker::stop(&record.container_id).await;
            return Err(SandboxError::Unavailable(format!(
                "Resume failed: firecracker sidecar for sandbox {} did not become healthy",
                record.id
            )));
        }
        let now = crate::util::now_ts();
        let _ = sandboxes()?.update(&record.id, |r| {
            r.state = SandboxState::Running;
            r.stopped_at = None;
            r.last_activity_at = now;
            r.sidecar_url = sidecar_url.clone();
            r.sidecar_port = sidecar_port;
        });
        return Ok(());
    }

    // For TEE-managed sandboxes, tee_deployment_id holds the real Docker container
    // ID (Direct backend) or cloud deployment ID (cloud backends). Use it for
    // Docker operations when available so the `tee-` prefixed container_id is
    // bypassed.
    let effective_container_id = record
        .tee_deployment_id
        .as_deref()
        .unwrap_or(&record.container_id);

    // Tier 1 (Hot): container still exists -> docker start
    if record.container_removed_at.is_none() {
        let builder = docker_builder().await?;
        let try_start = async {
            let mut container = docker_timeout(
                "load_container",
                Container::from_id(builder.client(), effective_container_id),
            )
            .await?;
            start_container_with_retry(&mut container).await?;
            Ok::<(), SandboxError>(())
        };
        match try_start.await {
            Ok(()) => {
                let (resumed_record, sidecar_ready) = match refresh_docker_sandbox_endpoint(record)
                    .await
                {
                    Ok(updated) => (updated, false),
                    Err(err) => {
                        blueprint_sdk::info!(
                            "resume: could not refresh port mapping for sandbox {}: {err}",
                            record.id
                        );
                        if wait_for_sidecar_health(&record.sidecar_url, 30).await {
                            blueprint_sdk::info!(
                                "resume: using stored sidecar URL for sandbox {} after refresh failure",
                                record.id
                            );
                            (record.clone(), true)
                        } else {
                            let _ =
                                stop_started_container(builder.client(), effective_container_id)
                                    .await;
                            return Err(SandboxError::Unavailable(format!(
                                "Resume failed: could not refresh sidecar URL for sandbox {}",
                                record.id
                            )));
                        }
                    }
                };

                if !sidecar_ready && !wait_for_sidecar_health(&resumed_record.sidecar_url, 30).await
                {
                    let _ = stop_started_container(builder.client(), effective_container_id).await;
                    return Err(SandboxError::Unavailable(format!(
                        "Resume failed: sidecar for sandbox {} did not become healthy at {}",
                        record.id, resumed_record.sidecar_url
                    )));
                }

                if resumed_record.ssh_port.is_some() {
                    let _ = restore_ssh_access(&resumed_record).await?;
                }

                let now = crate::util::now_ts();
                let _ = sandboxes()?.update(&record.id, |r| {
                    r.state = SandboxState::Running;
                    r.stopped_at = None;
                    r.last_activity_at = now;
                });
                return Ok(());
            }
            Err(err) => {
                blueprint_sdk::info!(
                    "resume: hot start failed for sandbox {}, trying warm: {err}",
                    record.id
                );
            }
        }
    }

    // Tier 2 (Warm): container gone, snapshot image exists -> create from image
    if record.snapshot_image_id.is_some() {
        create_from_snapshot_image(record).await?;
        return Ok(());
    }

    // Tier 3 (Cold): no image, S3 snapshot exists -> create from base + restore
    if record.snapshot_s3_url.is_some() {
        create_and_restore_from_s3(record).await?;
        return Ok(());
    }

    // Nothing available
    Err(SandboxError::Docker(format!(
        "Cannot resume sandbox {}: no container, snapshot image, or S3 snapshot available",
        record.id
    )))
}

/// Permanently destroy a sandbox, removing the container, image, and store entry.
///
/// For TEE-managed sandboxes, delegates to the TEE backend's `destroy()` method.
/// Accepts an explicit backend reference, or falls back to the global TEE backend.
pub async fn delete_sidecar(
    record: &SandboxRecord,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<()> {
    // If this is a TEE-managed sandbox, delegate to the backend.
    if let Some(deployment_id) = &record.tee_deployment_id {
        // Use explicit backend if provided, otherwise fall back to global.
        let backend = tee.map(Ok).unwrap_or_else(|| {
            crate::tee::try_tee_backend()
                .map(|b| b.as_ref())
                .ok_or_else(|| {
                    SandboxError::Validation(
                        "TEE sandbox has no backend available for deletion".into(),
                    )
                })
        })?;
        backend.destroy(deployment_id).await?;
        crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);
        return Ok(());
    }
    if record_uses_firecracker(record) {
        crate::firecracker::delete(&record.container_id).await?;
        crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);
        return Ok(());
    }
    // Default Docker removal path.
    delete_sidecar_docker(record).await
}

/// Recreate a sidecar container with updated user environment variables.
///
/// Stops and removes the old container, creates a new one with the
/// base env preserved and the provided `user_env_json` merged on top.
/// All other settings (image, CPU, memory, lifetime, token, agent identifier,
/// metadata, etc.) are faithfully preserved from the existing record.
///
/// Pass an empty string to clear user secrets (base env only).
///
/// Returns the new [`SandboxRecord`] for the recreated container.
pub async fn recreate_sidecar_with_env(
    sandbox_id: &str,
    user_env_json: &str,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<SandboxRecord> {
    let old = get_sandbox_by_id(sandbox_id)?;

    // TEE sandboxes cannot be recreated — it would invalidate attestation,
    // break sealed secrets, and orphan the on-chain deployment ID.
    if old.tee_deployment_id.is_some() {
        return Err(SandboxError::Validation(
            "Secret re-injection via container recreation is not supported for TEE sandboxes. \
             Use the sealed-secrets API instead."
                .into(),
        ));
    }

    // Stop if running, then delete
    if old.state == SandboxState::Running {
        let _ = stop_sidecar(&old).await;
    }
    delete_sidecar(&old, tee).await?;

    // Rebuild creation params faithfully from the stored record
    let image = if old.original_image.is_empty() {
        env::var("SIDECAR_IMAGE").unwrap_or_else(|_| DEFAULT_SIDECAR_IMAGE.to_string())
    } else {
        old.original_image.clone()
    };

    let old_token = old.token.clone();
    let params = CreateSandboxParams {
        name: old.name.clone(),
        image,
        stack: old.stack.clone(),
        agent_identifier: old.agent_identifier.clone(),
        env_json: old.base_env_json.clone(),
        user_env_json: user_env_json.to_string(),
        metadata_json: old.metadata_json.clone(),
        ssh_enabled: old.ssh_port.is_some(),
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: old.max_lifetime_seconds,
        idle_timeout_seconds: old.idle_timeout_seconds,
        cpu_cores: old.cpu_cores,
        memory_mb: old.memory_mb,
        disk_gb: if old.disk_gb > 0 { old.disk_gb } else { 10 },
        owner: old.owner.clone(),
        service_id: old.service_id,
        tee_config: old.tee_config.clone(),
        port_mappings: old.extra_ports.keys().copied().collect(),
    };

    // Preserve the original token so existing workflows/references keep working.
    let (_new_record, _attestation) =
        create_sidecar_with_token(&params, tee, Some(&old_token), Some(&old.id)).await?;
    let updated = sandboxes()?.update(&old.id, |record| {
        record.ssh_login_user = old.ssh_login_user.clone();
        record.ssh_authorized_keys = old.ssh_authorized_keys.clone();
    })?;
    if !updated {
        return Err(SandboxError::NotFound(format!(
            "Sandbox '{}' not found while restoring SSH state",
            old.id
        )));
    }
    if old.ssh_port.is_some() {
        restore_ssh_access(&get_sandbox_by_id(&old.id)?).await
    } else {
        Ok(get_sandbox_by_id(&old.id)?)
    }
}

async fn delete_sidecar_docker(record: &SandboxRecord) -> Result<()> {
    let builder = docker_builder().await?;
    let container = docker_timeout(
        "load_container",
        Container::from_id(builder.client(), &record.container_id),
    )
    .await?;
    docker_timeout(
        "remove_container",
        container.remove(Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        })),
    )
    .await?;

    crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);

    Ok(())
}

/// Docker-commit a stopped container to preserve filesystem state. Returns the image ID.
pub async fn commit_container(record: &SandboxRecord) -> Result<String> {
    if record_uses_firecracker(record) {
        return Err(SandboxError::Validation(
            "Snapshot image commit is not supported for runtime_backend=firecracker".into(),
        ));
    }
    let builder = docker_builder().await?;
    use docktopus::bollard::image::CommitContainerOptions;
    let options = CommitContainerOptions {
        container: record.container_id.clone(),
        repo: format!("sandbox-snapshot/{}", record.id),
        tag: "latest".to_string(),
        comment: format!("Auto-snapshot of sandbox {}", record.id),
        pause: true,
        ..Default::default()
    };
    let repo_tag = format!("sandbox-snapshot/{}:latest", record.id);
    let response = docker_timeout(
        "commit_container",
        builder
            .client()
            .commit_container(options, BollardConfig::<String>::default()),
    )
    .await?;
    Ok(response.id.filter(|s| !s.is_empty()).unwrap_or(repo_tag))
}

/// Remove a committed snapshot image from the local Docker daemon.
pub async fn remove_snapshot_image(image_id: &str) -> Result<()> {
    let builder = docker_builder().await?;
    docker_timeout(
        "remove_image",
        builder.client().remove_image(image_id, None, None),
    )
    .await?;
    Ok(())
}

/// Create a new container from a previously committed Docker image.
pub async fn create_from_snapshot_image(record: &SandboxRecord) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    let image_id = record
        .snapshot_image_id
        .as_deref()
        .ok_or_else(|| SandboxError::Docker("No snapshot image available".into()))?;

    let ssh_enabled = record.ssh_port.is_some();
    let effective_env = record.effective_env_json();
    let env_vars = build_env_vars(&effective_env, &record.token, config.container_port)?;
    let ep: Vec<u16> = record.extra_ports.keys().copied().collect();
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
        &ep,
    );

    let container_name = format!("sidecar-{}-warm", record.id);
    let mut container = Container::new(builder.client(), image_id.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
            "warm restore endpoint resolution",
            builder.client(),
            &container_id,
            config.container_port,
            ssh_enabled,
            &config.public_host,
            &record.extra_ports,
        )
        .await?;

        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            return Err(SandboxError::Unavailable(format!(
                "Resume failed: warm sidecar for sandbox {} did not become healthy at {}",
                record.id, sidecar_url
            )));
        }

        let now = crate::util::now_ts();
        let mut updated = record.clone();
        updated.container_id = container_id.clone();
        updated.sidecar_url = sidecar_url;
        updated.sidecar_port = sidecar_port;
        updated.ssh_port = ssh_port;
        updated.state = SandboxState::Running;
        updated.stopped_at = None;
        updated.last_activity_at = now;
        updated.container_removed_at = None;
        updated.snapshot_image_id = None;
        updated.extra_ports = extra_ports;

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        if ssh_enabled {
            restore_ssh_access(&updated).await
        } else {
            Ok(updated)
        }
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}

/// Create a fresh container from the original base image, then restore workspace from S3 snapshot.
pub async fn create_and_restore_from_s3(record: &SandboxRecord) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    let s3_url = record
        .snapshot_s3_url
        .as_deref()
        .ok_or_else(|| SandboxError::Docker("No S3 snapshot URL available".into()))?;

    let image = if record.original_image.is_empty() {
        &config.image
    } else {
        &record.original_image
    };

    ensure_image_pulled(&builder, image).await?;

    let ssh_enabled = record.ssh_port.is_some();
    let effective_env = record.effective_env_json();
    let env_vars = build_env_vars(&effective_env, &record.token, config.container_port)?;
    let ep: Vec<u16> = record.extra_ports.keys().copied().collect();
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
        &ep,
    );

    let container_name = format!("sidecar-{}-cold", record.id);
    let mut container = Container::new(builder.client(), image.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    start_container_with_retry(&mut container).await?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let finish = async {
        let (sidecar_url, sidecar_port, ssh_port, extra_ports) = refresh_port_mapping_with_retry(
            "cold restore endpoint resolution",
            builder.client(),
            &container_id,
            config.container_port,
            ssh_enabled,
            &config.public_host,
            &record.extra_ports,
        )
        .await?;
        let token = &record.token;

        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            return Err(SandboxError::Unavailable(format!(
                "Resume failed: cold sidecar for sandbox {} did not become healthy at {}",
                record.id, sidecar_url
            )));
        }

        // Restore workspace from S3 snapshot
        let restore_cmd = format!(
            "set -euo pipefail; curl -fsSL {} | tar -xzf - -C /",
            crate::util::shell_escape(s3_url)
        );
        let payload = serde_json::json!({
            "command": format!("sh -c {}", crate::util::shell_escape(&restore_cmd)),
        });
        if let Err(err) =
            crate::http::sidecar_post_json(&sidecar_url, "/terminals/commands", token, payload)
                .await
        {
            blueprint_sdk::error!("S3 restore failed for sandbox {}: {err}", record.id);
            return Err(SandboxError::Docker(format!("S3 restore failed: {err}")));
        }

        let now = crate::util::now_ts();
        let mut updated = record.clone();
        updated.container_id = container_id.clone();
        updated.sidecar_url = sidecar_url;
        updated.sidecar_port = sidecar_port;
        updated.ssh_port = ssh_port;
        updated.state = SandboxState::Running;
        updated.stopped_at = None;
        updated.last_activity_at = now;
        updated.container_removed_at = None;
        updated.image_removed_at = None;
        updated.extra_ports = extra_ports;
        updated.snapshot_s3_url = None;

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        if ssh_enabled {
            restore_ssh_access(&updated).await
        } else {
            Ok(updated)
        }
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(&builder, &container_id).await;
    }
    finish
}

/// Re-inspect a running container to get its current host port mappings.
///
/// After `docker stop` + `docker start`, Docker may assign new random host ports.
/// Returns `(sidecar_url, sidecar_port, ssh_port, extra_ports)`.
async fn refresh_port_mapping(
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
    container_port: u16,
    ssh_enabled: bool,
    public_host: &str,
    prev_extra_ports: &HashMap<u16, u16>,
) -> Result<(String, u16, Option<u16>, HashMap<u16, u16>)> {
    use docktopus::bollard::container::InspectContainerOptions;
    let inspect = docker_timeout(
        "inspect_container",
        client.inspect_container(container_id, None::<InspectContainerOptions>),
    )
    .await?;
    let use_host_network =
        std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");
    let (sidecar_port, ssh_port, extra) = if use_host_network {
        (container_port, None, HashMap::new())
    } else {
        let (sp, ssh) = extract_ports(&inspect, container_port, ssh_enabled)?;
        let container_ports: Vec<u16> = prev_extra_ports.keys().copied().collect();
        let extra = extract_extra_ports(&inspect, &container_ports);
        (sp, ssh, extra)
    };
    let sidecar_url = format!("http://{public_host}:{sidecar_port}");
    Ok((sidecar_url, sidecar_port, ssh_port, extra))
}

fn extract_ports(
    inspect: &docktopus::bollard::models::ContainerInspectResponse,
    container_port: u16,
    ssh_enabled: bool,
) -> Result<(u16, Option<u16>)> {
    let network = inspect
        .network_settings
        .as_ref()
        .and_then(|settings| settings.ports.as_ref())
        .ok_or_else(|| SandboxError::Docker("Missing container port mappings".into()))?;

    let sidecar_port = extract_host_port(network, container_port)?;
    let ssh_port = if ssh_enabled {
        Some(extract_host_port(network, DEFAULT_SIDECAR_SSH_PORT)?)
    } else {
        None
    };

    Ok((sidecar_port, ssh_port))
}

fn extract_host_port(
    ports: &HashMap<String, Option<Vec<PortBinding>>>,
    container_port: u16,
) -> Result<u16> {
    let key = format!("{container_port}/tcp");
    let bindings = ports
        .get(&key)
        .and_then(|v| v.as_ref())
        .ok_or_else(|| SandboxError::Docker(format!("Missing port bindings for {key}")))?;
    let host_port = bindings
        .first()
        .and_then(|binding| binding.host_port.as_ref())
        .ok_or_else(|| SandboxError::Docker(format!("Missing host port for {key}")))?;
    let parsed = host_port
        .parse::<u16>()
        .map_err(|_| SandboxError::Docker(format!("Invalid host port for {key}")))?;
    if parsed == 0 {
        return Err(SandboxError::Docker(format!(
            "Host port for {key} is not assigned yet"
        )));
    }
    Ok(parsed)
}

/// Parse extra port mappings from metadata_json and explicit port_mappings field.
///
/// Ports come from two sources, deduplicated and capped at [`MAX_EXTRA_PORTS`]:
/// 1. `metadata_json` field `"ports"` — a JSON array of port numbers
/// 2. `CreateSandboxParams.port_mappings` — explicit list
///
/// Reserved ports (sidecar HTTP, SSH, and well-known system ports < 1) are excluded.
fn parse_extra_ports(metadata_json: &str, explicit: &[u16]) -> Vec<u16> {
    use crate::MAX_EXTRA_PORTS;
    let config = SidecarRuntimeConfig::load();
    let reserved = [config.container_port, config.ssh_port];

    let mut ports: Vec<u16> = Vec::new();

    // From metadata_json.ports
    if let Ok(Some(meta)) = parse_json_object(metadata_json, "metadata_json") {
        if let Some(arr) = meta.get("ports").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(p) = v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                    ports.push(p);
                }
            }
        }
    }

    // From explicit field
    ports.extend_from_slice(explicit);

    // Deduplicate, filter reserved, cap
    ports.sort_unstable();
    ports.dedup();
    ports.retain(|p| *p > 0 && !reserved.contains(p));
    ports.truncate(MAX_EXTRA_PORTS);
    ports
}

/// Extract host port mappings for extra user ports from a container inspect result.
///
/// Returns a map of container_port → host_port for each port that was successfully
/// bound. Ports that Docker failed to map are silently skipped.
fn extract_extra_ports(
    inspect: &docktopus::bollard::models::ContainerInspectResponse,
    container_ports: &[u16],
) -> HashMap<u16, u16> {
    let network = match inspect
        .network_settings
        .as_ref()
        .and_then(|s| s.ports.as_ref())
    {
        Some(n) => n,
        None => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for &cp in container_ports {
        if let Ok(hp) = extract_host_port(network, cp) {
            map.insert(cp, hp);
        }
    }
    map
}

#[cfg(test)]
mod port_mapping_tests {
    use super::*;

    static INIT: std::sync::Once = std::sync::Once::new();

    fn init() {
        INIT.call_once(|| unsafe {
            std::env::set_var("SIDECAR_IMAGE", "test:latest");
            std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        });
    }

    #[test]
    fn parse_ports_from_metadata_json() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [3000, 5432, 9090]}"#, &[]);
        assert_eq!(ports, vec![3000, 5432, 9090]);
    }

    #[test]
    fn parse_ports_from_explicit_field() {
        init();
        let ports = parse_extra_ports("{}", &[3000, 5432]);
        assert_eq!(ports, vec![3000, 5432]);
    }

    #[test]
    fn parse_ports_deduplicates() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [3000, 5432]}"#, &[3000, 9090]);
        assert_eq!(ports, vec![3000, 5432, 9090]);
    }

    #[test]
    fn parse_ports_filters_reserved_sidecar_port() {
        init();
        let config = SidecarRuntimeConfig::load();
        let ports = parse_extra_ports(
            &format!(r#"{{"ports": [{}, 3000]}}"#, config.container_port),
            &[],
        );
        // Sidecar port (8080) should be filtered out
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_filters_reserved_ssh_port() {
        init();
        let config = SidecarRuntimeConfig::load();
        let ports = parse_extra_ports(&format!(r#"{{"ports": [{}, 3000]}}"#, config.ssh_port), &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_filters_zero() {
        init();
        let ports = parse_extra_ports(r#"{"ports": [0, 3000]}"#, &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_caps_at_max() {
        init();
        let all: Vec<u16> = (3000..3020).collect();
        let ports = parse_extra_ports("{}", &all);
        assert_eq!(ports.len(), crate::MAX_EXTRA_PORTS);
    }

    #[test]
    fn parse_ports_empty_metadata() {
        init();
        let ports = parse_extra_ports("{}", &[]);
        assert!(ports.is_empty());
    }

    #[test]
    fn parse_ports_invalid_metadata() {
        init();
        let ports = parse_extra_ports("not-json", &[3000]);
        // Should still parse explicit ports even if metadata is invalid
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn parse_ports_ignores_non_numeric() {
        init();
        let ports = parse_extra_ports(r#"{"ports": ["not-a-port", 3000, true]}"#, &[]);
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn build_docker_config_includes_extra_ports() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, false, 1, 512, None, &[3000, 5432]);

        let exposed = docker_config.exposed_ports.unwrap();
        assert!(exposed.contains_key("3000/tcp"));
        assert!(exposed.contains_key("5432/tcp"));
        assert!(exposed.contains_key(&format!("{}/tcp", config.container_port)));

        let bindings = docker_config.host_config.unwrap().port_bindings.unwrap();
        assert!(bindings.contains_key("3000/tcp"));
        assert!(bindings.contains_key("5432/tcp"));
    }

    #[test]
    fn build_docker_config_no_extra_ports() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, false, 1, 512, None, &[]);

        let exposed = docker_config.exposed_ports.unwrap();
        // Only sidecar port should be exposed (no SSH since ssh_enabled=false)
        assert_eq!(exposed.len(), 1);
        assert!(exposed.contains_key(&format!("{}/tcp", config.container_port)));
    }

    #[test]
    fn build_docker_config_adds_ssh_caps_when_enabled() {
        init();
        let config = SidecarRuntimeConfig::load();
        let docker_config = build_docker_config(config, true, 1, 512, None, &[]);

        let caps = docker_config.host_config.unwrap().cap_add.unwrap();
        assert!(caps.contains(&"CHOWN".to_string()));
        assert!(caps.contains(&"NET_BIND_SERVICE".to_string()));
        assert!(caps.contains(&"SYS_CHROOT".to_string()));
    }

    #[test]
    fn docker_ssh_bootstrap_unlocks_login_user() {
        let command = build_docker_ssh_bootstrap_command("agent");
        assert!(command.contains("passwd -u \"$user\""));
        assert!(command.contains("AllowUsers agent"));
    }

    #[test]
    fn select_docker_ssh_login_user_prefers_sidecar_then_agent() {
        let selected = select_docker_ssh_login_user(|candidate| candidate == "agent");
        assert_eq!(selected, Some("agent"));

        let selected = select_docker_ssh_login_user(|candidate| {
            candidate == SSH_DEFAULT_LOGIN_USER || candidate == SSH_FALLBACK_LOGIN_USER
        });
        assert_eq!(selected, Some(SSH_DEFAULT_LOGIN_USER));
    }

    #[test]
    fn select_docker_ssh_login_user_returns_none_when_no_compatible_user_exists() {
        let selected = select_docker_ssh_login_user(|_| false);
        assert_eq!(selected, None);
    }

    #[test]
    fn extra_ports_serde_roundtrip() {
        let mut ports = HashMap::new();
        ports.insert(3000u16, 32768u16);
        ports.insert(5432, 32769);

        let json = serde_json::to_string(&ports).unwrap();
        let restored: HashMap<u16, u16> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.get(&3000), Some(&32768));
        assert_eq!(restored.get(&5432), Some(&32769));
    }

    #[test]
    fn extra_ports_default_empty_on_deserialize() {
        // Simulates loading a record from before extra_ports existed
        let json = r#"{"id":"test","container_id":"c","sidecar_url":"http://x","sidecar_port":0,"token":"t","created_at":0}"#;
        let record: SandboxRecord = serde_json::from_str(json).unwrap();
        assert!(record.extra_ports.is_empty());
    }
}

#[cfg(test)]
mod runtime_backend_tests {
    use super::*;

    fn params(metadata_json: &str) -> CreateSandboxParams {
        CreateSandboxParams {
            metadata_json: metadata_json.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn parse_runtime_backend_aliases() {
        assert_eq!(
            parse_runtime_backend_value("docker"),
            Some(RuntimeBackend::Docker)
        );
        assert_eq!(
            parse_runtime_backend_value("container"),
            Some(RuntimeBackend::Docker)
        );
        assert_eq!(
            parse_runtime_backend_value("firecracker"),
            Some(RuntimeBackend::Firecracker)
        );
        assert_eq!(
            parse_runtime_backend_value("microvm"),
            Some(RuntimeBackend::Firecracker)
        );
        assert_eq!(
            parse_runtime_backend_value("tee"),
            Some(RuntimeBackend::Tee)
        );
        assert_eq!(
            parse_runtime_backend_value("confidential-vm"),
            Some(RuntimeBackend::Tee)
        );
        assert_eq!(parse_runtime_backend_value("unknown"), None);
    }

    #[test]
    fn resolve_runtime_backend_from_metadata() {
        let resolved = resolve_runtime_backend(&params(r#"{"runtime_backend":"firecracker"}"#));
        assert_eq!(resolved.unwrap(), RuntimeBackend::Firecracker);

        let resolved_nested =
            resolve_runtime_backend(&params(r#"{"runtime":{"backend":"tee"}}"#)).unwrap();
        assert_eq!(resolved_nested, RuntimeBackend::Tee);
    }

    #[test]
    fn resolve_runtime_backend_forces_tee_when_required() {
        let mut request = params(r#"{"runtime_backend":"docker"}"#);
        request.tee_config = Some(crate::tee::TeeConfig {
            required: true,
            tee_type: crate::tee::TeeType::Tdx,
            attestation_nonce: None,
        });
        let resolved = resolve_runtime_backend(&request).unwrap();
        assert_eq!(resolved, RuntimeBackend::Tee);
    }

    #[test]
    fn resolve_runtime_backend_rejects_firecracker_plus_tee_required() {
        let mut request = params(r#"{"runtime_backend":"firecracker"}"#);
        request.tee_config = Some(crate::tee::TeeConfig {
            required: true,
            tee_type: crate::tee::TeeType::Tdx,
            attestation_nonce: None,
        });
        let err = resolve_runtime_backend(&request).unwrap_err().to_string();
        assert!(err.contains("incompatible"));
    }
}

#[cfg(test)]
mod tee_tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init() {
        INIT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("runtime-tee-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe {
                std::env::set_var("BLUEPRINT_STATE_DIR", dir.to_str().unwrap());
                std::env::set_var("SIDECAR_IMAGE", "test:latest");
                std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
            }
        });
    }

    fn tee_required_params() -> CreateSandboxParams {
        CreateSandboxParams {
            name: "tee-test".into(),
            image: "test:latest".into(),
            tee_config: Some(crate::tee::TeeConfig {
                required: true,
                tee_type: crate::tee::TeeType::Tdx,
                attestation_nonce: None,
            }),
            owner: "0xabcdef".into(),
            cpu_cores: 2,
            memory_mb: 4096,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_sidecar_tee_required_no_backend() {
        init();
        let params = tee_required_params();
        let result = create_sidecar(&params, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("TEE runtime selected but no TEE backend configured"),
            "unexpected: {err}"
        );
    }

    #[tokio::test]
    async fn create_sidecar_tee_success() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);
        let params = tee_required_params();

        let (record, attestation) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Record should have TEE fields
        assert!(record.tee_deployment_id.is_some());
        assert!(record.container_id.starts_with("tee-"));
        assert!(record.sidecar_url.starts_with("http://mock-tee:"));
        assert!(record.tee_metadata_json.is_some());
        assert!(record.tee_config.is_some());
        assert_eq!(record.owner, "0xabcdef");
        assert_eq!(record.cpu_cores, 2);
        assert_eq!(record.memory_mb, 4096);

        // Attestation should be present
        let att = attestation.unwrap();
        assert_eq!(att.tee_type, crate::tee::TeeType::Tdx);

        // Mock should have been called
        assert_eq!(
            mock.deploy_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn create_sidecar_tee_stores_record() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Nitro);
        let mut params = tee_required_params();
        params.tee_config.as_mut().unwrap().tee_type = crate::tee::TeeType::None;

        let (record, _) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Verify the record is in the store
        let stored = sandboxes().unwrap().get(&record.id).unwrap().unwrap();
        assert_eq!(stored.id, record.id);
        assert_eq!(stored.tee_deployment_id, record.tee_deployment_id);
        assert!(stored.container_id.starts_with("tee-"));
    }

    #[tokio::test]
    async fn create_sidecar_tee_deploy_failure() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::failing(crate::tee::TeeType::Tdx);
        let params = tee_required_params();

        let result = create_sidecar(&params, Some(&mock)).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Mock deploy failure")
        );
    }

    #[tokio::test]
    async fn delete_sidecar_tee_calls_destroy() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);

        // First create a TEE sandbox
        let params = tee_required_params();
        let (record, _) = create_sidecar(&params, Some(&mock)).await.unwrap();

        // Now delete it
        delete_sidecar(&record, Some(&mock)).await.unwrap();
        assert_eq!(
            mock.destroy_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn create_sidecar_non_tee_skips_mock() {
        init();
        let mock = crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx);
        let params = CreateSandboxParams {
            name: "docker-test".into(),
            image: "test:latest".into(),
            tee_config: None, // no TEE
            ..Default::default()
        };

        // This will try Docker (and fail since no Docker in tests), but
        // the mock's deploy should NOT be called.
        let _ = create_sidecar(&params, Some(&mock)).await;
        assert_eq!(
            mock.deploy_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "Mock deploy should not be called for non-TEE requests"
        );
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn seal_unseal_roundtrip() {
        let plaintext = "super-secret-token-123";
        let sealed = seal_field(plaintext).unwrap();
        assert!(sealed.starts_with(ENC_PREFIX), "should have enc prefix");
        assert_ne!(sealed, plaintext);

        let unsealed = unseal_field(&sealed).unwrap();
        assert_eq!(unsealed, plaintext);
    }

    #[test]
    fn unseal_plaintext_passthrough() {
        let plain = "not-encrypted-token";
        let result = unseal_field(plain).unwrap();
        assert_eq!(result, plain, "plaintext should pass through unchanged");
    }

    #[test]
    fn seal_empty_string() {
        let sealed = seal_field("").unwrap();
        assert_eq!(sealed, "", "empty string should stay empty");
        let unsealed = unseal_field("").unwrap();
        assert_eq!(unsealed, "", "empty unseal should stay empty");
    }

    #[test]
    fn seal_record_roundtrip() {
        let mut record = SandboxRecord {
            id: "test".into(),
            container_id: "ctr".into(),
            sidecar_url: "http://x".into(),
            sidecar_port: 0,
            ssh_port: None,
            token: "my-token".into(),
            created_at: 0,
            cpu_cores: 0,
            memory_mb: 0,
            state: SandboxState::Running,
            idle_timeout_seconds: 0,
            max_lifetime_seconds: 0,
            last_activity_at: 0,
            stopped_at: None,
            snapshot_image_id: None,
            snapshot_s3_url: None,
            container_removed_at: None,
            image_removed_at: None,
            original_image: String::new(),
            base_env_json: r#"{"KEY":"val"}"#.into(),
            user_env_json: r#"{"USER":"x"}"#.into(),
            snapshot_destination: None,
            tee_deployment_id: None,
            tee_metadata_json: None,
            tee_attestation_json: None,
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
            service_id: None,
            tee_config: None,
            extra_ports: HashMap::new(),
            ssh_login_user: None,
            ssh_authorized_keys: Vec::new(),
        };

        seal_record(&mut record).unwrap();
        assert!(record.token.starts_with(ENC_PREFIX));
        assert!(record.base_env_json.starts_with(ENC_PREFIX));
        assert!(record.user_env_json.starts_with(ENC_PREFIX));

        unseal_record(&mut record).unwrap();
        assert_eq!(record.token, "my-token");
        assert_eq!(record.base_env_json, r#"{"KEY":"val"}"#);
        assert_eq!(record.user_env_json, r#"{"USER":"x"}"#);
    }

    #[test]
    fn unseal_corrupted_ciphertext_returns_error() {
        // Valid prefix but garbage base64 payload (nonce + corrupted ciphertext)
        let corrupted = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD
                .encode(b"123456789012XXXX_corrupted_data_here")
        );
        let result = unseal_field(&corrupted);
        assert!(result.is_err(), "corrupted ciphertext should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("decrypt failed"),
            "error should mention decrypt failure: {err_msg}"
        );
    }

    #[test]
    fn seal_produces_different_ciphertext_each_time() {
        let plaintext = "determinism-test";
        let sealed1 = seal_field(plaintext).unwrap();
        let sealed2 = seal_field(plaintext).unwrap();
        assert_ne!(
            sealed1, sealed2,
            "each seal call should use a random nonce, producing different output"
        );

        // Both should decrypt back to the same plaintext
        assert_eq!(unseal_field(&sealed1).unwrap(), plaintext);
        assert_eq!(unseal_field(&sealed2).unwrap(), plaintext);
    }

    #[test]
    fn unseal_short_ciphertext_returns_error() {
        // Prefix present but payload too short to contain a 12-byte nonce
        let short = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(b"short")
        );
        let result = unseal_field(&short);
        assert!(result.is_err(), "too-short ciphertext should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("too short"),
            "error should mention 'too short': {err_msg}"
        );
    }

    #[test]
    fn unseal_invalid_base64_returns_error() {
        let bad = format!("{ENC_PREFIX}!!!not-valid-base64!!!");
        let result = unseal_field(&bad);
        assert!(result.is_err(), "invalid base64 should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("base64"),
            "error should mention base64: {err_msg}"
        );
    }

    #[test]
    fn seal_large_value() {
        // 1 MB plaintext — verifies no size-related panics or truncation
        let plaintext = "A".repeat(1024 * 1024);
        let sealed = seal_field(&plaintext).unwrap();
        assert!(sealed.starts_with(ENC_PREFIX), "should have enc prefix");
        // Ciphertext + nonce + base64 overhead makes it larger
        assert!(
            sealed.len() > plaintext.len(),
            "sealed form should be larger than plaintext"
        );

        let unsealed = unseal_field(&sealed).unwrap();
        assert_eq!(
            unsealed.len(),
            plaintext.len(),
            "unsealed length should match original"
        );
        assert_eq!(unsealed, plaintext, "unsealed value should match original");
    }

    #[test]
    fn unseal_tampered_ciphertext() {
        // Seal a real value, then flip a byte in the ciphertext portion
        let plaintext = "sensitive-data-that-must-not-silently-corrupt";
        let sealed = seal_field(plaintext).unwrap();

        // Decode, tamper, re-encode
        let encoded = &sealed[ENC_PREFIX.len()..];
        let mut blob = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        // Flip a byte in the ciphertext portion (past the 12-byte nonce)
        assert!(blob.len() > 13, "blob should be longer than nonce");
        blob[13] ^= 0xFF;
        let tampered = format!(
            "{ENC_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(&blob)
        );

        let result = unseal_field(&tampered);
        assert!(
            result.is_err(),
            "tampered ciphertext must fail authentication, not return corrupted data"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("decrypt failed"),
            "error should mention decrypt failure: {err_msg}"
        );
    }
}

#[cfg(test)]
mod core_logic_tests {
    use super::*;
    use docktopus::bollard::models::{ContainerInspectResponse, NetworkSettings, PortBinding};

    // ── effective_idle_timeout ───────────────────────────────────────────

    fn test_config() -> SidecarRuntimeConfig {
        SidecarRuntimeConfig {
            image: "test".into(),
            public_host: "127.0.0.1".into(),
            container_port: 3000,
            ssh_port: 2222,
            timeout: Duration::from_secs(30),
            docker_host: None,
            pull_image: false,
            sandbox_default_idle_timeout: 1800,
            sandbox_default_max_lifetime: 86400,
            sandbox_max_idle_timeout: 7200,
            sandbox_max_max_lifetime: 172800,
            sandbox_reaper_interval: 30,
            sandbox_gc_interval: 3600,
            sandbox_gc_hot_retention: 86400,
            sandbox_gc_warm_retention: 172800,
            sandbox_gc_cold_retention: 604800,
            snapshot_auto_commit: true,
            snapshot_destination_prefix: None,
            sandbox_max_count: 100,
        }
    }

    #[test]
    fn adjusted_sandbox_count_reuses_existing_slot() {
        assert_eq!(adjusted_sandbox_count_for_limit(0, false), 0);
        assert_eq!(adjusted_sandbox_count_for_limit(1, false), 1);
        assert_eq!(adjusted_sandbox_count_for_limit(1, true), 0);
        assert_eq!(adjusted_sandbox_count_for_limit(5, true), 4);
    }

    #[test]
    fn effective_idle_timeout_zero_and_clamped() {
        let cfg = test_config();
        assert_eq!(cfg.effective_idle_timeout(0), 1800, "zero → default");
        assert_eq!(
            cfg.effective_idle_timeout(99999),
            7200,
            "over max → clamped"
        );
        assert_eq!(
            cfg.effective_idle_timeout(3600),
            3600,
            "in range → passthrough"
        );
    }

    #[test]
    fn effective_max_lifetime_zero_and_clamped() {
        let cfg = test_config();
        assert_eq!(cfg.effective_max_lifetime(0), 86400, "zero → default");
        assert_eq!(
            cfg.effective_max_lifetime(999999),
            172800,
            "over max → clamped"
        );
        assert_eq!(
            cfg.effective_max_lifetime(100000),
            100000,
            "in range → passthrough"
        );
    }

    // ── build_env_vars ──────────────────────────────────────────────────

    #[test]
    fn env_vars_with_json() {
        let vars = build_env_vars(r#"{"API_KEY":"sk-test","DEBUG":"true"}"#, "tok", 8080).unwrap();
        assert!(vars.contains(&"API_KEY=sk-test".to_string()));
        assert!(vars.contains(&"DEBUG=true".to_string()));
        assert!(vars.contains(&"SIDECAR_PORT=8080".to_string()));
    }

    #[test]
    fn env_vars_invalid_json() {
        let result = build_env_vars("not-json", "tok", 3000);
        assert!(result.is_err());
    }

    #[test]
    fn env_vars_preserve_explicit_ai_env() {
        let vars = build_env_vars(r#"{"ZAI_API_KEY":"user-key"}"#, "tok", 8080).unwrap();
        assert!(vars.contains(&"ZAI_API_KEY=user-key".to_string()));
        assert!(!vars.contains(&"OPENCODE_MODEL_API_KEY=user-key".to_string()));
    }

    #[test]
    fn workflow_runtime_credentials_available_requires_sandbox_env() {
        assert!(!workflow_runtime_credentials_available("{}").unwrap());
    }

    #[test]
    fn workflow_runtime_credentials_available_rejects_incomplete_explicit_ai_env() {
        let old = std::env::var("ZAI_API_KEY").ok();
        // SAFETY: test scopes environment mutation and restores the prior value.
        unsafe {
            std::env::set_var("ZAI_API_KEY", "operator-key");
        }
        assert!(
            !workflow_runtime_credentials_available(
                r#"{"OPENCODE_MODEL_PROVIDER":"zai-coding-plan"}"#
            )
            .unwrap()
        );

        // SAFETY: restore previous process environment for the next test.
        unsafe {
            match old {
                Some(value) => std::env::set_var("ZAI_API_KEY", value),
                None => std::env::remove_var("ZAI_API_KEY"),
            }
        }
    }

    // ── extract_host_port ───────────────────────────────────────────────

    fn make_port_map(port: u16, host_port: &str) -> HashMap<String, Option<Vec<PortBinding>>> {
        let mut map = HashMap::new();
        map.insert(
            format!("{port}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some(host_port.to_string()),
            }]),
        );
        map
    }

    #[test]
    fn extract_host_port_valid() {
        let ports = make_port_map(3000, "32768");
        let result = extract_host_port(&ports, 3000).unwrap();
        assert_eq!(result, 32768);
    }

    #[test]
    fn extract_host_port_missing_port() {
        let ports = make_port_map(3000, "32768");
        let result = extract_host_port(&ports, 8080);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_invalid_number() {
        let ports = make_port_map(3000, "not-a-number");
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_zero_is_not_ready() {
        let ports = make_port_map(3000, "0");
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    #[test]
    fn extract_host_port_empty_bindings() {
        let mut ports: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        ports.insert("3000/tcp".to_string(), Some(vec![]));
        let result = extract_host_port(&ports, 3000);
        assert!(result.is_err());
    }

    // ── extract_ports (full) ────────────────────────────────────────────

    fn make_inspect(
        port_map: HashMap<String, Option<Vec<PortBinding>>>,
    ) -> ContainerInspectResponse {
        ContainerInspectResponse {
            network_settings: Some(NetworkSettings {
                ports: Some(port_map),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extract_ports_no_ssh() {
        let ports = make_port_map(3000, "49000");
        let inspect = make_inspect(ports);
        let (sidecar, ssh) = extract_ports(&inspect, 3000, false).unwrap();
        assert_eq!(sidecar, 49000);
        assert!(ssh.is_none());
    }

    #[test]
    fn extract_ports_with_ssh() {
        let mut ports = make_port_map(3000, "49000");
        ports.insert(
            format!("{DEFAULT_SIDECAR_SSH_PORT}/tcp"),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some("49001".to_string()),
            }]),
        );
        let inspect = make_inspect(ports);
        let (sidecar, ssh) = extract_ports(&inspect, 3000, true).unwrap();
        assert_eq!(sidecar, 49000);
        assert_eq!(ssh, Some(49001));
    }

    #[test]
    fn extract_ports_missing_network() {
        let inspect = ContainerInspectResponse {
            network_settings: None,
            ..Default::default()
        };
        let result = extract_ports(&inspect, 3000, false);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_retries_until_success() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = retry_port_mapping_lookup_inner("test resolution", "ctr-1", 3, 0, {
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();
                async move {
                    let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if attempt < 2 {
                        Err(SandboxError::Docker(
                            "Host port for 3000/tcp is not assigned yet".into(),
                        ))
                    } else {
                        Ok(49000u16)
                    }
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(result, 49000);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_stops_on_non_retryable_error() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result =
            retry_port_mapping_lookup_inner::<u16, _, _>("test resolution", "ctr-2", 3, 0, {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(SandboxError::Docker(
                            "Failed to connect to Docker: daemon unavailable".into(),
                        ))
                    }
                }
            })
            .await;

        let err = result.expect_err("expected non-retryable error to bubble up");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            err.to_string().contains("daemon unavailable"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn retry_port_mapping_lookup_inner_wraps_exhausted_transient_error() {
        let result = retry_port_mapping_lookup_inner::<u16, _, _>(
            "test resolution",
            "ctr-3",
            2,
            0,
            || async {
                Err(SandboxError::Docker(
                    "Missing host port for 3000/tcp".into(),
                ))
            },
        )
        .await;

        let err = result.expect_err("expected retries to exhaust");
        assert!(
            err.to_string().contains(
                "test resolution failed: Docker did not publish sidecar port for container ctr-3 after 2 attempts"
            ),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("Missing host port for 3000/tcp"),
            "unexpected error: {err}"
        );
    }

    // ── SandboxState ────────────────────────────────────────────────────

    #[test]
    fn sandbox_state_default_is_running() {
        assert_eq!(SandboxState::default(), SandboxState::Running);
    }

    #[test]
    fn sandbox_state_serialization_roundtrip() {
        let running = serde_json::to_string(&SandboxState::Running).unwrap();
        let stopped = serde_json::to_string(&SandboxState::Stopped).unwrap();
        assert_eq!(
            serde_json::from_str::<SandboxState>(&running).unwrap(),
            SandboxState::Running
        );
        assert_eq!(
            serde_json::from_str::<SandboxState>(&stopped).unwrap(),
            SandboxState::Stopped
        );
    }
}
