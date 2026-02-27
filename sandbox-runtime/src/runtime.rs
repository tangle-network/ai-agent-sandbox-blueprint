use docktopus::DockerBuilder;
use docktopus::bollard::container::{
    Config as BollardConfig, InspectContainerOptions, RemoveContainerOptions,
};
use docktopus::bollard::models::{HostConfig, PortBinding, PortMap};
use docktopus::container::Container;
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::OnceCell as AsyncOnceCell;

use crate::error::{Result, SandboxError};
use crate::util::{merge_metadata, parse_json_object};
use crate::{DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE, DEFAULT_SIDECAR_SSH_PORT};

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
    Ok(instance_store()?.get("instance")?.map(|mut r| {
        unseal_record(&mut r);
        r
    }))
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
    Err(last_err.unwrap())
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

    if stored.is_empty() || !stored.starts_with(ENC_PREFIX) {
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
pub fn seal_record(record: &mut SandboxRecord) {
    match seal_field(&record.token) {
        Ok(sealed) => record.token = sealed,
        Err(e) => {
            tracing::error!(field = "token", error = %e, "Failed to encrypt field — storing plaintext")
        }
    }
    match seal_field(&record.base_env_json) {
        Ok(sealed) => record.base_env_json = sealed,
        Err(e) => {
            tracing::error!(field = "base_env_json", error = %e, "Failed to encrypt field — storing plaintext")
        }
    }
    match seal_field(&record.user_env_json) {
        Ok(sealed) => record.user_env_json = sealed,
        Err(e) => {
            tracing::error!(field = "user_env_json", error = %e, "Failed to encrypt field — storing plaintext")
        }
    }
}

/// Decrypt sensitive fields in a `SandboxRecord` after reading from store.
pub fn unseal_record(record: &mut SandboxRecord) {
    match unseal_field(&record.token) {
        Ok(plain) => record.token = plain,
        Err(e) => {
            tracing::error!(field = "token", error = %e, "Failed to decrypt field — returning raw value")
        }
    }
    match unseal_field(&record.base_env_json) {
        Ok(plain) => record.base_env_json = plain,
        Err(e) => {
            tracing::error!(field = "base_env_json", error = %e, "Failed to decrypt field — returning raw value")
        }
    }
    match unseal_field(&record.user_env_json) {
        Ok(plain) => record.user_env_json = plain,
        Err(e) => {
            tracing::error!(field = "user_env_json", error = %e, "Failed to decrypt field — returning raw value")
        }
    }
}

fn next_sandbox_id() -> String {
    format!("sandbox-{}", uuid::Uuid::new_v4())
}

pub fn get_sandbox_by_id(id: &str) -> Result<SandboxRecord> {
    let mut record = sandboxes()?
        .get(id)?
        .ok_or_else(|| SandboxError::NotFound(format!("Sandbox '{id}' not found")))?;
    unseal_record(&mut record);
    Ok(record)
}

pub fn get_sandbox_by_url(sidecar_url: &str) -> Result<SandboxRecord> {
    let url = sidecar_url.to_string();
    let mut record = sandboxes()?
        .find(|record| record.sidecar_url == url)?
        .ok_or_else(|| {
            SandboxError::NotFound(format!("Sandbox not found for URL: {sidecar_url}"))
        })?;
    unseal_record(&mut record);
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
            .map(|mut r| {
                unseal_record(&mut r);
                r
            })
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
    create_sidecar_with_token(request, tee, None).await
}

/// Internal: create sidecar with optional token override.
async fn create_sidecar_with_token(
    request: &CreateSandboxParams,
    tee: Option<&dyn crate::tee::TeeBackend>,
    token_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    // Route to TEE backend if TEE is required.
    if let Some(config) = &request.tee_config {
        if config.required {
            let backend = tee.ok_or_else(|| {
                SandboxError::Validation("TEE required but no backend configured".into())
            })?;
            return create_sidecar_tee(request, backend, token_override).await;
        }
    }
    // Default Docker path.
    create_sidecar_docker(request, token_override)
        .await
        .map(|r| (r, None))
}

async fn create_sidecar_tee(
    request: &CreateSandboxParams,
    backend: &dyn crate::tee::TeeBackend,
    token_override: Option<&str>,
) -> Result<(SandboxRecord, Option<crate::tee::AttestationReport>)> {
    let config = SidecarRuntimeConfig::load();
    let sandbox_id = next_sandbox_id();
    let token = match token_override {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => crate::auth::generate_token(),
    };

    let tee_params = crate::tee::TeeDeployParams::from_sandbox_params(
        &sandbox_id,
        request,
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
        name: request.name.clone(),
        agent_identifier: request.agent_identifier.clone(),
        metadata_json: request.metadata_json.clone(),
        disk_gb: request.disk_gb,
        stack: request.stack.clone(),
        owner: request.owner.clone(),
        tee_config: request.tee_config.clone(),
    };

    let mut sealed = record.clone();
    seal_record(&mut sealed);
    sandboxes()?.insert(sandbox_id, sealed)?;
    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok((record, Some(deployment.attestation)))
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

    let mut exposed_ports = HashMap::new();
    exposed_ports.insert(format!("{}/tcp", config.container_port), HashMap::new());
    if ssh_enabled {
        exposed_ports.insert(format!("{}/tcp", config.ssh_port), HashMap::new());
    }

    let mut host_config = HostConfig {
        port_bindings: Some(port_bindings),
        cap_drop: Some(vec!["ALL".to_string()]),
        cap_add: Some(vec!["SYS_PTRACE".to_string()]),
        security_opt: Some(vec!["no-new-privileges=true".to_string()]),
        pids_limit: Some(512),
        readonly_rootfs: Some(true),
        tmpfs: Some(HashMap::from([
            ("/tmp".to_string(), "rw,noexec,nosuid,size=512m".to_string()),
            ("/run".to_string(), "rw,noexec,nosuid,size=64m".to_string()),
        ])),
        ..Default::default()
    };
    if cpu_cores > 0 {
        host_config.nano_cpus = Some((cpu_cores as i64) * 1_000_000_000);
    }
    if memory_mb > 0 {
        host_config.memory = Some((memory_mb as i64) * 1024 * 1024);
    }

    BollardConfig {
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        labels,
        ..Default::default()
    }
}

async fn create_sidecar_docker(
    request: &CreateSandboxParams,
    token_override: Option<&str>,
) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    ensure_image_pulled(builder, &config.image).await?;

    let sandbox_id = next_sandbox_id();
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

    let override_config = build_docker_config(
        config,
        request.ssh_enabled,
        request.cpu_cores,
        request.memory_mb,
        labels,
    );

    let mut container = Container::new(builder.client(), config.image.clone())
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

        let (sidecar_port, ssh_port) =
            extract_ports(&inspect, config.container_port, request.ssh_enabled)?;
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
            original_image: config.image.clone(),
            base_env_json: request.env_json.clone(),
            user_env_json: request.user_env_json.clone(),
            snapshot_destination,
            tee_deployment_id: None,
            tee_metadata_json: None,
            name: request.name.clone(),
            agent_identifier: request.agent_identifier.clone(),
            metadata_json: request.metadata_json.clone(),
            disk_gb: request.disk_gb,
            stack: request.stack.clone(),
            owner: request.owner.clone(),
            tee_config: None,
        };

        let mut sealed = record.clone();
        seal_record(&mut sealed);
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
pub async fn stop_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Stopped {
        return Err(SandboxError::Validation(
            "Sandbox is already stopped".into(),
        ));
    }
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

/// Resume a stopped sandbox, restoring from container, snapshot image, or S3 as available.
pub async fn resume_sidecar(record: &SandboxRecord) -> Result<()> {
    if record.state == SandboxState::Running {
        return Err(SandboxError::Validation(
            "Sandbox is already running".into(),
        ));
    }
    // Tier 1 (Hot): container still exists -> docker start
    if record.container_removed_at.is_none() {
        let builder = docker_builder().await?;
        let try_start = async {
            let mut container = docker_timeout(
                "load_container",
                Container::from_id(builder.client(), &record.container_id),
            )
            .await?;
            start_container_with_retry(&mut container).await?;
            Ok::<(), SandboxError>(())
        };
        match try_start.await {
            Ok(()) => {
                // Re-read port mappings — Docker may assign new host ports after restart.
                let config = SidecarRuntimeConfig::load();
                let sidecar_url = match refresh_port_mapping(
                    builder.client(),
                    &record.container_id,
                    config.container_port,
                    record.ssh_port.is_some(),
                    &config.public_host,
                )
                .await
                {
                    Ok((url, sidecar_port, ssh_port)) => {
                        let now = crate::util::now_ts();
                        let _ = sandboxes()?.update(&record.id, |r| {
                            r.state = SandboxState::Running;
                            r.stopped_at = None;
                            r.last_activity_at = now;
                            r.sidecar_url = url.clone();
                            r.sidecar_port = sidecar_port;
                            r.ssh_port = ssh_port;
                        });
                        url
                    }
                    Err(err) => {
                        blueprint_sdk::info!(
                            "resume: could not refresh port mapping for sandbox {}: {err}",
                            record.id
                        );
                        // Fall back to stored URL
                        let now = crate::util::now_ts();
                        let _ = sandboxes()?.update(&record.id, |r| {
                            r.state = SandboxState::Running;
                            r.stopped_at = None;
                            r.last_activity_at = now;
                        });
                        record.sidecar_url.clone()
                    }
                };

                if !wait_for_sidecar_health(&sidecar_url, 30).await {
                    blueprint_sdk::info!(
                        "resume: hot start sidecar slow to respond for sandbox {}",
                        record.id
                    );
                }
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
        let updated = create_from_snapshot_image(record).await?;
        if !wait_for_sidecar_health(&updated.sidecar_url, 30).await {
            blueprint_sdk::info!(
                "resume: warm start sidecar slow to respond for sandbox {}",
                record.id
            );
        }
        return Ok(());
    }

    // Tier 3 (Cold): no image, S3 snapshot exists -> create from base + restore
    if record.snapshot_s3_url.is_some() {
        let updated = create_and_restore_from_s3(record).await?;
        let _ = updated;
        return Ok(());
    }

    // Nothing available
    Err(SandboxError::Docker(format!(
        "Cannot resume sandbox {}: no container, snapshot image, or S3 snapshot available",
        record.id
    )))
}

/// Permanently destroy a sandbox, removing the container, image, and store entry.
pub async fn delete_sidecar(
    record: &SandboxRecord,
    tee: Option<&dyn crate::tee::TeeBackend>,
) -> Result<()> {
    // If this is a TEE-managed sandbox, delegate to the backend.
    if let Some(deployment_id) = &record.tee_deployment_id {
        if let Some(backend) = tee {
            backend.destroy(deployment_id).await?;
            crate::metrics::metrics().record_sandbox_deleted(record.cpu_cores, record.memory_mb);
            return Ok(());
        }
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
    };

    // Preserve the original token so existing workflows/references keep working.
    let (new_record, _attestation) =
        create_sidecar_with_token(&params, tee, Some(&old_token)).await?;
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
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
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

        let (sidecar_port, ssh_port) = extract_ports(&inspect, config.container_port, ssh_enabled)?;
        let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);

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
        seal_record(&mut sealed);
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
    let override_config = build_docker_config(
        config,
        ssh_enabled,
        record.cpu_cores,
        record.memory_mb,
        None,
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

        let (sidecar_port, ssh_port) = extract_ports(&inspect, config.container_port, ssh_enabled)?;
        let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);
        let token = &record.token;

        if !wait_for_sidecar_health(&sidecar_url, 30).await {
            blueprint_sdk::info!(
                "S3 restore: sidecar slow to start, proceeding with restore anyway"
            );
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
        seal_record(&mut sealed);
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
/// Returns `(sidecar_url, sidecar_port, ssh_port)`.
async fn refresh_port_mapping(
    client: std::sync::Arc<docktopus::bollard::Docker>,
    container_id: &str,
    container_port: u16,
    ssh_enabled: bool,
    public_host: &str,
) -> Result<(String, u16, Option<u16>)> {
    use docktopus::bollard::container::InspectContainerOptions;
    let inspect = docker_timeout(
        "inspect_container",
        client.inspect_container(container_id, None::<InspectContainerOptions>),
    )
    .await?;
    let (sidecar_port, ssh_port) = extract_ports(&inspect, container_port, ssh_enabled)?;
    let sidecar_url = format!("http://{public_host}:{sidecar_port}");
    Ok((sidecar_url, sidecar_port, ssh_port))
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
    host_port
        .parse::<u16>()
        .map_err(|_| SandboxError::Docker(format!("Invalid host port for {key}")))
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
            err.contains("TEE required but no backend configured"),
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
            name: String::new(),
            agent_identifier: String::new(),
            metadata_json: String::new(),
            disk_gb: 0,
            stack: String::new(),
            owner: String::new(),
            tee_config: None,
        };

        seal_record(&mut record);
        assert!(record.token.starts_with(ENC_PREFIX));
        assert!(record.base_env_json.starts_with(ENC_PREFIX));
        assert!(record.user_env_json.starts_with(ENC_PREFIX));

        unseal_record(&mut record);
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
