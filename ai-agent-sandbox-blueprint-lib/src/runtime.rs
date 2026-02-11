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

use crate::SandboxCreateRequest;
use crate::auth::token_from_request;
use crate::error::{Result, SandboxError};
use crate::util::{merge_metadata, parse_json_object};
use crate::{DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE, DEFAULT_SIDECAR_SSH_PORT};

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
                    // Backward compat: fall back to old env var name
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SandboxState {
    Running,
    Stopped,
}

impl Default for SandboxState {
    fn default() -> Self {
        SandboxState::Running
    }
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
    #[serde(default)]
    pub env_json: String,
    #[serde(default)]
    pub snapshot_destination: Option<String>,
}

use crate::store::PersistentStore;

static SANDBOXES: OnceCell<PersistentStore<SandboxRecord>> = OnceCell::new();
static DOCKER_BUILDER: AsyncOnceCell<DockerBuilder> = AsyncOnceCell::const_new();
static IMAGE_PULLED: AsyncOnceCell<()> = AsyncOnceCell::const_new();

pub fn sandboxes() -> Result<&'static PersistentStore<SandboxRecord>> {
    SANDBOXES
        .get_or_try_init(|| {
            let path = crate::store::state_dir().join("sandboxes.json");
            PersistentStore::open(path)
        })
        .map_err(|err: SandboxError| err)
}

pub async fn docker_builder() -> Result<&'static DockerBuilder> {
    DOCKER_BUILDER
        .get_or_try_init(|| async {
            let config = SidecarRuntimeConfig::load();
            let builder = match config.docker_host.as_deref() {
                Some(host) => DockerBuilder::with_address(host).await.map_err(|err| {
                    SandboxError::Docker(format!("Failed to connect to docker at {host}: {err}"))
                })?,
                None => DockerBuilder::new().await.map_err(|err| {
                    SandboxError::Docker(format!("Failed to connect to docker: {err}"))
                })?,
            };
            Ok(builder)
        })
        .await
}

fn next_sandbox_id() -> String {
    format!("sandbox-{}", uuid::Uuid::new_v4())
}

pub fn get_sandbox_by_id(id: &str) -> Result<SandboxRecord> {
    sandboxes()?
        .get(id)?
        .ok_or_else(|| SandboxError::NotFound(format!("Sandbox '{id}' not found")))
}

pub fn get_sandbox_by_url(sidecar_url: &str) -> Result<SandboxRecord> {
    let url = sidecar_url.to_string();
    sandboxes()?
        .find(|record| record.sidecar_url == url)?
        .ok_or_else(|| SandboxError::NotFound(format!("Sandbox not found for URL: {sidecar_url}")))
}

/// Update `last_activity_at` to now for the given sandbox.
pub fn touch_sandbox(sandbox_id: &str) {
    if let Ok(store) = sandboxes() {
        let now = crate::workflows::now_ts();
        let _ = store.update(sandbox_id, |r| {
            r.last_activity_at = now;
        });
    }
}

/// Find a sandbox by its sidecar URL, returning `None` instead of an error if not found.
pub fn get_sandbox_by_url_opt(sidecar_url: &str) -> Option<SandboxRecord> {
    let url = sidecar_url.to_string();
    sandboxes()
        .ok()
        .and_then(|store| store.find(|record| record.sidecar_url == url).ok().flatten())
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
async fn ensure_image_pulled(builder: &DockerBuilder, image: &str) -> Result<()> {
    IMAGE_PULLED
        .get_or_try_init(|| async {
            let config = SidecarRuntimeConfig::load();
            if config.pull_image {
                builder.pull_image(image, None).await.map_err(|err| {
                    SandboxError::Docker(format!("Failed to pull image {image}: {err}"))
                })?;
            }
            Ok::<(), SandboxError>(())
        })
        .await?;
    Ok(())
}

pub async fn create_sidecar(request: &SandboxCreateRequest) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    ensure_image_pulled(builder, &config.image).await?;

    let sandbox_id = next_sandbox_id();
    let token = token_from_request(request.sidecar_token.as_str());
    let container_name = format!("sidecar-{sandbox_id}");

    let mut env_vars = Vec::new();
    env_vars.push(format!("SIDECAR_PORT={}", config.container_port));
    env_vars.push(format!("SIDECAR_AUTH_TOKEN={token}"));

    if !request.env_json.trim().is_empty() {
        let env_map = parse_json_object(&request.env_json, "env_json")?;
        if let Some(Value::Object(map)) = env_map {
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

    let mut port_bindings = PortMap::new();
    port_bindings.insert(
        format!("{}/tcp", config.container_port),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: None,
        }]),
    );

    if request.ssh_enabled {
        port_bindings.insert(
            format!("{}/tcp", config.ssh_port),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: None,
            }]),
        );
    }

    let mut exposed_ports = HashMap::new();
    exposed_ports.insert(format!("{}/tcp", config.container_port), HashMap::new());
    if request.ssh_enabled {
        exposed_ports.insert(format!("{}/tcp", config.ssh_port), HashMap::new());
    }

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

    // Build resource constraints from request
    let mut host_config = HostConfig {
        port_bindings: Some(port_bindings),
        ..Default::default()
    };

    if request.cpu_cores > 0 {
        // Docker expects NanoCPUs (1 core = 1_000_000_000 nanoCPUs)
        host_config.nano_cpus = Some((request.cpu_cores as i64) * 1_000_000_000);
    }

    if request.memory_mb > 0 {
        // Docker expects bytes
        host_config.memory = Some((request.memory_mb as i64) * 1024 * 1024);
    }

    let override_config = BollardConfig {
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        labels,
        ..Default::default()
    };

    let mut container = Container::new(builder.client(), config.image.clone())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    container
        .start(false)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to start sidecar container: {err}")))?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let inspect = builder
        .client()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to inspect container: {err}")))?;

    let (sidecar_port, ssh_port) =
        extract_ports(&inspect, config.container_port, request.ssh_enabled)?;
    let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);

    let now = crate::workflows::now_ts();
    let idle_timeout = config.effective_idle_timeout(request.idle_timeout_seconds);
    let max_lifetime = config.effective_max_lifetime(request.max_lifetime_seconds);

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id,
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
        env_json: request.env_json.clone(),
        snapshot_destination,
    };

    sandboxes()?.insert(sandbox_id, record.clone())?;

    crate::metrics::metrics().record_sandbox_created(request.cpu_cores, request.memory_mb);

    Ok(record)
}

pub async fn stop_sidecar(record: &SandboxRecord) -> Result<()> {
    let builder = docker_builder().await?;
    let mut container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to load container: {err}")))?;
    container
        .stop()
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to stop container: {err}")))?;

    let now = crate::workflows::now_ts();
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
            if let Ok(resp) = crate::util::http_client().and_then(|c| Ok(c.get(&url))) {
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

pub async fn resume_sidecar(record: &SandboxRecord) -> Result<()> {
    // Tier 1 (Hot): container still exists → docker start
    if record.container_removed_at.is_none() {
        let builder = docker_builder().await?;
        let try_start = async {
            let mut container = Container::from_id(builder.client(), &record.container_id)
                .await
                .map_err(|err| SandboxError::Docker(format!("Failed to load container: {err}")))?;
            container
                .start(false)
                .await
                .map_err(|err| SandboxError::Docker(format!("Failed to start container: {err}")))?;
            Ok::<(), SandboxError>(())
        };
        match try_start.await {
            Ok(()) => {
                let now = crate::workflows::now_ts();
                let _ = sandboxes()?.update(&record.id, |r| {
                    r.state = SandboxState::Running;
                    r.stopped_at = None;
                    r.last_activity_at = now;
                });
                if !wait_for_sidecar_health(&record.sidecar_url, 30).await {
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

    // Tier 2 (Warm): container gone, snapshot image exists → create from image
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

    // Tier 3 (Cold): no image, S3 snapshot exists → create from base + restore
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

pub async fn delete_sidecar(record: &SandboxRecord) -> Result<()> {
    let builder = docker_builder().await?;
    let container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to load container: {err}")))?;
    container
        .remove(Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        }))
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to remove container: {err}")))?;

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
    let response = builder
        .client()
        .commit_container(options, BollardConfig::<String>::default())
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to commit container: {err}")))?;
    Ok(response.id.unwrap_or_default())
}

/// Remove a committed snapshot image from the local Docker daemon.
pub async fn remove_snapshot_image(image_id: &str) -> Result<()> {
    let builder = docker_builder().await?;
    builder
        .client()
        .remove_image(image_id, None, None)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to remove image {image_id}: {err}")))?;
    Ok(())
}

/// Create a new container from a previously committed Docker image.
///
/// Reuses the original env vars, port bindings, and resource constraints stored in the record.
pub async fn create_from_snapshot_image(record: &SandboxRecord) -> Result<SandboxRecord> {
    let config = SidecarRuntimeConfig::load();
    let builder = docker_builder().await?;

    let image_id = record
        .snapshot_image_id
        .as_deref()
        .ok_or_else(|| SandboxError::Docker("No snapshot image available".into()))?;

    let mut env_vars = Vec::new();
    env_vars.push(format!("SIDECAR_PORT={}", config.container_port));
    env_vars.push(format!("SIDECAR_AUTH_TOKEN={}", record.token));

    if !record.env_json.trim().is_empty() {
        if let Ok(Some(Value::Object(map))) = crate::util::parse_json_object(&record.env_json, "env_json") {
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

    let mut port_bindings = PortMap::new();
    port_bindings.insert(
        format!("{}/tcp", config.container_port),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: None,
        }]),
    );
    let ssh_enabled = record.ssh_port.is_some();
    if ssh_enabled {
        port_bindings.insert(
            format!("{}/tcp", config.ssh_port),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
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
        ..Default::default()
    };
    if record.cpu_cores > 0 {
        host_config.nano_cpus = Some((record.cpu_cores as i64) * 1_000_000_000);
    }
    if record.memory_mb > 0 {
        host_config.memory = Some((record.memory_mb as i64) * 1024 * 1024);
    }

    let override_config = BollardConfig {
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        labels: None,
        ..Default::default()
    };

    let container_name = format!("sidecar-{}-warm", record.id);
    let mut container = Container::new(builder.client(), image_id.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    container
        .start(false)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to start from snapshot image: {err}")))?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let inspect = builder
        .client()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to inspect container: {err}")))?;

    let (sidecar_port, ssh_port) = extract_ports(&inspect, config.container_port, ssh_enabled)?;
    let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);

    let now = crate::workflows::now_ts();
    let mut updated = record.clone();
    updated.container_id = container_id;
    updated.sidecar_url = sidecar_url;
    updated.sidecar_port = sidecar_port;
    updated.ssh_port = ssh_port;
    updated.state = SandboxState::Running;
    updated.stopped_at = None;
    updated.last_activity_at = now;
    updated.container_removed_at = None;
    updated.snapshot_image_id = None;

    sandboxes()?.insert(record.id.clone(), updated.clone())?;
    Ok(updated)
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

    let mut env_vars = Vec::new();
    env_vars.push(format!("SIDECAR_PORT={}", config.container_port));
    env_vars.push(format!("SIDECAR_AUTH_TOKEN={}", record.token));

    if !record.env_json.trim().is_empty() {
        if let Ok(Some(Value::Object(map))) = crate::util::parse_json_object(&record.env_json, "env_json") {
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

    let mut port_bindings = PortMap::new();
    port_bindings.insert(
        format!("{}/tcp", config.container_port),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: None,
        }]),
    );
    let ssh_enabled = record.ssh_port.is_some();
    if ssh_enabled {
        port_bindings.insert(
            format!("{}/tcp", config.ssh_port),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
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
        ..Default::default()
    };
    if record.cpu_cores > 0 {
        host_config.nano_cpus = Some((record.cpu_cores as i64) * 1_000_000_000);
    }
    if record.memory_mb > 0 {
        host_config.memory = Some((record.memory_mb as i64) * 1024 * 1024);
    }

    let override_config = BollardConfig {
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        labels: None,
        ..Default::default()
    };

    let container_name = format!("sidecar-{}-cold", record.id);
    let mut container = Container::new(builder.client(), image.to_string())
        .with_name(container_name)
        .env(env_vars)
        .config_override(override_config);

    container
        .start(false)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to start from base image: {err}")))?;

    let container_id = container
        .id()
        .ok_or_else(|| SandboxError::Docker("Missing container id".into()))?
        .to_string();

    let inspect = builder
        .client()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to inspect container: {err}")))?;

    let (sidecar_port, ssh_port) = extract_ports(&inspect, config.container_port, ssh_enabled)?;
    let sidecar_url = format!("http://{}:{}", config.public_host, sidecar_port);
    let token = &record.token;

    if !wait_for_sidecar_health(&sidecar_url, 30).await {
        blueprint_sdk::info!("S3 restore: sidecar slow to start, proceeding with restore anyway");
    }

    // Restore workspace from S3 snapshot
    let restore_cmd = format!(
        "set -euo pipefail; curl -fsSL {} | tar -xzf - -C /",
        crate::util::shell_escape(s3_url)
    );
    let payload = serde_json::json!({
        "command": format!("sh -c {}", crate::util::shell_escape(&restore_cmd)),
    });
    if let Err(err) = crate::http::sidecar_post_json(&sidecar_url, "/terminals/commands", token, payload).await {
        blueprint_sdk::error!("S3 restore failed for sandbox {}: {err}", record.id);
        return Err(SandboxError::Docker(format!("S3 restore failed: {err}")));
    }

    let now = crate::workflows::now_ts();
    let mut updated = record.clone();
    updated.container_id = container_id;
    updated.sidecar_url = sidecar_url;
    updated.sidecar_port = sidecar_port;
    updated.ssh_port = ssh_port;
    updated.state = SandboxState::Running;
    updated.stopped_at = None;
    updated.last_activity_at = now;
    updated.container_removed_at = None;
    updated.image_removed_at = None;
    updated.snapshot_s3_url = None;

    sandboxes()?.insert(record.id.clone(), updated.clone())?;
    Ok(updated)
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
