use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions,
};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use once_cell::sync::OnceCell;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::env;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::OnceCell as AsyncOnceCell;

use crate::error::{Result, SandboxError};
use crate::util::{merge_metadata, parse_json_object};
use crate::{DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE, DEFAULT_SIDECAR_SSH_PORT};

const RESUME_PORT_MAPPING_RETRY_ATTEMPTS: usize = 20;
const RESUME_PORT_MAPPING_RETRY_DELAY_MS: u64 = 500;

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
    pub web_terminal_enabled: bool,
    pub max_lifetime_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
    /// On-chain caller address (hex string, e.g. "0x1234..."). Set by the job
    /// handler from the `Caller` extractor so that ownership can be enforced.
    pub owner: String,
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
            let docker_host = env::var("DOCKER_HOST").ok();
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
    /// TEE configuration used to create this sandbox (preserved for recreation).
    #[serde(default)]
    pub tee_config: Option<crate::tee::TeeConfig>,
    /// Extra user-requested port mappings: container_port → host_port.
    /// Populated from `metadata_json.ports` at creation time.
    #[serde(default)]
    pub extra_ports: HashMap<u16, u16>,
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
static DOCKER_BUILDER: AsyncOnceCell<DockerBuilder> = AsyncOnceCell::const_new();
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

/// Return the cached Docker client, connecting on first call.
pub async fn docker_builder() -> Result<&'static DockerBuilder> {
    // Return cached builder if already initialized.
    if let Some(builder) = DOCKER_BUILDER.get() {
        return Ok(builder);
    }
    // Build a new connection. If this fails, the error is returned but NOT
    // cached, so subsequent calls will retry instead of being permanently
    // broken by a transient Docker outage.
    let config = SidecarRuntimeConfig::load();
    let builder = match config.docker_host.as_deref() {
        Some(host) => DockerBuilder::with_address(host).await.map_err(|err| {
            SandboxError::Docker(format!("Failed to connect to Docker at {host}: {err}"))
        })?,
        None => DockerBuilder::new()
            .await
            .map_err(|err| SandboxError::Docker(format!("Failed to connect to Docker: {err}")))?,
    };
    // If another task raced us, use theirs; otherwise cache ours.
    Ok(DOCKER_BUILDER.get_or_init(|| async { builder }).await)
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
async fn create_sidecar_with_token(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    match resolve_runtime_backend(request)? {
        RuntimeBackend::Tee => {
            let backend = tee.ok_or_else(|| {
                SandboxError::Validation(
                    "TEE runtime selected but no TEE backend configured".into(),
                )
            })?;
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
        tee_config: request.tee_config.clone(),
        extra_ports: deployment.extra_ports,
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

fn parse_url_port(url: &str) -> Option<u16> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.port_or_known_default())
}

async fn create_sidecar_firecracker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
    sandbox_id_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();

    if config.sandbox_max_count > 0 {
        let current = sandboxes()?.values()?.len();
        if current >= config.sandbox_max_count {
            return Err(SandboxError::Validation(format!(
                "Sandbox limit reached ({current}/{max}). Delete unused sandboxes before creating new ones.",
                max = config.sandbox_max_count,
            )));
        }
    }

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

    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);

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
        tee_config: None,
        extra_ports: HashMap::new(),
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

/// Build the `Vec<String>` of `KEY=VALUE` env vars for a Docker container.
fn build_env_vars(env_json: &str, token: &str, container_port: u16) -> Result<Vec<String>> {
    let mut env_vars = vec![
        format!("SIDECAR_PORT={container_port}"),
        format!("SIDECAR_AUTH_TOKEN={token}"),
    ];
    if !env_json.trim().is_empty() {
        if let Some(Value::Object(map)) = parse_json_object(env_json, "env_json")? {
            for (key, value) in map {
                let val = match value {
                    Value::String(v) => v,
                    Value::Number(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    _ => continue,
                };
                env_vars.push(format!("{key}={val}"));
            }
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
        cap_add: Some(vec![
            "SYS_PTRACE".to_string(),
            "SETGID".to_string(),
            "SETUID".to_string(),
        ]),
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

    // Enforce per-operator sandbox count limit to prevent resource exhaustion.
    if config.sandbox_max_count > 0 {
        let current = sandboxes()?.values()?.len();
        if current >= config.sandbox_max_count {
            return Err(SandboxError::Validation(format!(
                "Sandbox limit reached ({current}/{max}). Delete unused sandboxes before creating new ones.",
                max = config.sandbox_max_count,
            )));
        }
    }

    let builder = docker_builder().await?;

    // Use the user-supplied image if provided, otherwise fall back to the
    // operator's SIDECAR_IMAGE env var.
    let effective_image = if request.image.is_empty() {
        config.image.clone()
    } else {
        request.image.clone()
    };

    ensure_image_pulled(builder, &effective_image).await?;
    let original_image = effective_image.clone();

    let sandbox_id = sandbox_id_override
        .map(ToString::to_string)
        .unwrap_or_else(next_sandbox_id);
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
        let inspect = docker_timeout(
            "inspect_container",
            builder
                .client()
                .inspect_container(&container_id, None::<InspectContainerOptions>),
        )
        .await?;

        let use_host_network =
            std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");
        let (sidecar_port, ssh_port, extra_port_map) = if use_host_network {
            // Host network mode: container ports bind directly on the host.
            // No Docker port mappings — use the container's internal ports.
            (config.container_port, None, HashMap::new())
        } else {
            let (sp, ssh) = extract_ports(&inspect, config.container_port, request.ssh_enabled)?;
            let epm = extract_extra_ports(&inspect, &extra_ports);
            (sp, ssh, epm)
        };
        let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);

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
            tee_config: None,
            extra_ports: extra_port_map,
        };

        let mut sealed = record.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(sandbox_id, sealed)?;

        crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

        Ok(record)
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(builder, &container_id).await;
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
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
    container_port: u16,
    ssh_enabled: bool,
    public_host: &str,
    prev_extra_ports: &HashMap<u16, u16>,
) -> Result<(String, u16, Option<u16>, HashMap<u16, u16>)> {
    let mut last_err = None;

    for attempt in 0..RESUME_PORT_MAPPING_RETRY_ATTEMPTS {
        match refresh_port_mapping(
            client.clone(),
            container_id,
            container_port,
            ssh_enabled,
            public_host,
            prev_extra_ports,
        )
        .await
        {
            Ok(mapping) => return Ok(mapping),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < RESUME_PORT_MAPPING_RETRY_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(
                        RESUME_PORT_MAPPING_RETRY_DELAY_MS,
                    ))
                    .await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        SandboxError::Unavailable(format!(
            "Unable to refresh port mapping for container {container_id}"
        ))
    }))
}

async fn stop_started_container(
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
) -> Result<()> {
    let mut container = docker_timeout(
        "load_container",
        Container::from_id(client, container_id),
    )
    .await?;
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
                // Re-read port mappings — Docker may assign new host ports after restart.
                let config = SidecarRuntimeConfig::load();
                let (sidecar_url, sidecar_port, ssh_port, extra_ports) =
                    match refresh_port_mapping_with_retry(
                        builder.client(),
                        effective_container_id,
                        config.container_port,
                        record.ssh_port.is_some(),
                        &config.public_host,
                        &record.extra_ports,
                    )
                    .await
                    {
                        Ok(mapping) => mapping,
                        Err(err) => {
                            blueprint_sdk::info!(
                                "resume: could not refresh port mapping for sandbox {}: {err}",
                                record.id
                            );
                            let _ = stop_started_container(builder.client(), effective_container_id).await;
                            return Err(SandboxError::Unavailable(format!(
                                "Resume failed: could not refresh sidecar URL for sandbox {}",
                                record.id
                            )));
                        }
                    };

                if !wait_for_sidecar_health(&sidecar_url, 30).await {
                    let _ = stop_started_container(builder.client(), effective_container_id).await;
                    return Err(SandboxError::Unavailable(format!(
                        "Resume failed: sidecar for sandbox {} did not become healthy at {}",
                        record.id, sidecar_url
                    )));
                }

                let now = crate::util::now_ts();
                let _ = sandboxes()?.update(&record.id, |r| {
                    r.state = SandboxState::Running;
                    r.stopped_at = None;
                    r.last_activity_at = now;
                    r.sidecar_url = sidecar_url;
                    r.sidecar_port = sidecar_port;
                    r.ssh_port = ssh_port;
                    r.extra_ports = extra_ports;
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
    sandboxes()?.remove(sandbox_id)?;

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
        tee_config: old.tee_config.clone(),
        port_mappings: old.extra_ports.keys().copied().collect(),
    };

    // Preserve the original token so existing workflows/references keep working.
    let (new_record, _attestation) =
        create_sidecar_with_token(&params, tee, Some(&old_token), Some(&old.id)).await?;
    Ok(new_record)
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
        let inspect = docker_timeout(
            "inspect_container",
            builder
                .client()
                .inspect_container(&container_id, None::<InspectContainerOptions>),
        )
        .await?;

        let use_host_network =
            std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");
        let (sidecar_port, ssh_port) = if use_host_network {
            (config.container_port, None)
        } else {
            extract_ports(&inspect, config.container_port, ssh_enabled)?
        };
        let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);

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

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        Ok(updated)
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(builder, &container_id).await;
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

    ensure_image_pulled(builder, image).await?;

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
        let inspect = docker_timeout(
            "inspect_container",
            builder
                .client()
                .inspect_container(&container_id, None::<InspectContainerOptions>),
        )
        .await?;

        let use_host_network =
            std::env::var("SIDECAR_NETWORK_HOST").is_ok_and(|v| v == "true" || v == "1");
        let (sidecar_port, ssh_port) = if use_host_network {
            (config.container_port, None)
        } else {
            extract_ports(&inspect, config.container_port, ssh_enabled)?
        };
        let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);
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
        updated.snapshot_s3_url = None;

        let mut sealed = updated.clone();
        seal_record(&mut sealed)?;
        sandboxes()?.insert(record.id.clone(), sealed)?;
        Ok(updated)
    }
    .await;

    if finish.is_err() {
        cleanup_orphaned_container(builder, &container_id).await;
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
        let params = tee_required_params();

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
            tee_config: None,
            extra_ports: HashMap::new(),
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
