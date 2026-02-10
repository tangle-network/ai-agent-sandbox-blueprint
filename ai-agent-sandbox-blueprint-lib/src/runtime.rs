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
}

static RUNTIME_CONFIG: OnceCell<SidecarRuntimeConfig> = OnceCell::new();

impl SidecarRuntimeConfig {
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

            SidecarRuntimeConfig {
                image,
                public_host,
                container_port,
                ssh_port,
                timeout: Duration::from_secs(timeout),
                docker_host,
                pull_image,
            }
        })
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

    let record = SandboxRecord {
        id: sandbox_id.clone(),
        container_id,
        sidecar_url,
        sidecar_port,
        ssh_port,
        token,
        created_at: crate::workflows::now_ts(),
        cpu_cores: request.cpu_cores,
        memory_mb: request.memory_mb,
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
    Ok(())
}

pub async fn resume_sidecar(record: &SandboxRecord) -> Result<()> {
    let builder = docker_builder().await?;
    let mut container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to load container: {err}")))?;
    container
        .start(false)
        .await
        .map_err(|err| SandboxError::Docker(format!("Failed to start container: {err}")))?;
    Ok(())
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
