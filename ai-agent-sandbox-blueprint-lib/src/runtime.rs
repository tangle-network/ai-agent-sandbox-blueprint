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
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::OnceCell as AsyncOnceCell;

use crate::auth::token_from_request;
use crate::util::{merge_metadata, parse_json_object};
use crate::{
    DEFAULT_SIDECAR_HTTP_PORT, DEFAULT_SIDECAR_IMAGE, DEFAULT_SIDECAR_SSH_PORT,
    SandboxCreateRequest,
};

#[derive(Clone, Debug)]
pub struct SidecarRuntimeConfig {
    pub image: String,
    pub public_host: String,
    pub container_port: u16,
    pub ssh_port: u16,
    pub timeout: Duration,
    pub docker_host: Option<String>,
    pub pull_image: bool,
    pub mock_sidecar_url: Option<String>,
}

impl SidecarRuntimeConfig {
    pub fn load() -> Self {
        let image = env::var("SIDECAR_IMAGE").unwrap_or_else(|_| DEFAULT_SIDECAR_IMAGE.to_string());
        let public_host =
            env::var("SIDECAR_PUBLIC_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let container_port = env::var("SIDECAR_HTTP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_SIDECAR_HTTP_PORT);
        let ssh_port = env::var("SIDECAR_SSH_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_SIDECAR_SSH_PORT);
        let timeout = env::var("REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(crate::DEFAULT_TIMEOUT_SECS);
        let docker_host = env::var("DOCKER_HOST").ok();
        let pull_image = env::var("SIDECAR_PULL_IMAGE")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true);
        let mock_sidecar_url = env::var("SIDECAR_MOCK_URL").ok();

        SidecarRuntimeConfig {
            image,
            public_host,
            container_port,
            ssh_port,
            timeout: Duration::from_secs(timeout),
            docker_host,
            pull_image,
            mock_sidecar_url,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SandboxRecord {
    pub id: String,
    pub container_id: String,
    pub sidecar_url: String,
    pub sidecar_port: u16,
    pub ssh_port: Option<u16>,
    pub token: String,
    pub created_at: u64,
}

static SANDBOX_COUNTER: AtomicU64 = AtomicU64::new(1);
static SANDBOXES: OnceCell<Mutex<HashMap<String, SandboxRecord>>> = OnceCell::new();
static DOCKER_BUILDER: AsyncOnceCell<DockerBuilder> = AsyncOnceCell::const_new();

pub fn sandboxes() -> Result<&'static Mutex<HashMap<String, SandboxRecord>>, String> {
    SANDBOXES
        .get_or_try_init(|| Ok(Mutex::new(HashMap::new())))
        .map_err(|err: String| err)
}

pub async fn docker_builder() -> Result<&'static DockerBuilder, String> {
    DOCKER_BUILDER
        .get_or_try_init(|| async {
            let config = SidecarRuntimeConfig::load();
            let builder = match config.docker_host.as_deref() {
                Some(host) => DockerBuilder::with_address(host)
                    .await
                    .map_err(|err| format!("Failed to connect to docker at {host}: {err}"))?,
                None => DockerBuilder::new()
                    .await
                    .map_err(|err| format!("Failed to connect to docker: {err}"))?,
            };
            Ok(builder)
        })
        .await
        .map_err(|err: String| err)
}

pub fn next_sandbox_id() -> String {
    let id = SANDBOX_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sandbox-{id}")
}

pub fn get_sandbox_by_id(id: &str) -> Result<SandboxRecord, String> {
    let store = sandboxes()?
        .lock()
        .map_err(|_| "Sandbox store poisoned".to_string())?;
    store
        .get(id)
        .cloned()
        .ok_or_else(|| "Sandbox not found".to_string())
}

pub fn get_sandbox_by_url(sidecar_url: &str) -> Result<SandboxRecord, String> {
    let store = sandboxes()?
        .lock()
        .map_err(|_| "Sandbox store poisoned".to_string())?;
    store
        .values()
        .find(|record| record.sidecar_url == sidecar_url)
        .cloned()
        .ok_or_else(|| "Sandbox not found for sidecar_url".to_string())
}

pub fn require_sidecar_auth(sidecar_url: &str, token: &str) -> Result<SandboxRecord, String> {
    let record = get_sandbox_by_url(sidecar_url)?;
    if record.token != token {
        return Err("Unauthorized sidecar_token".to_string());
    }
    Ok(record)
}

pub async fn create_sidecar(request: &SandboxCreateRequest) -> Result<SandboxRecord, String> {
    let config = SidecarRuntimeConfig::load();
    if let Some(mock_url) = config.mock_sidecar_url.clone() {
        let sandbox_id = next_sandbox_id();
        let token = token_from_request(request.sidecar_token.as_str());
        let record = SandboxRecord {
            id: sandbox_id.clone(),
            container_id: "mock".to_string(),
            sidecar_url: mock_url,
            sidecar_port: config.container_port,
            ssh_port: if request.ssh_enabled {
                Some(config.ssh_port)
            } else {
                None
            },
            token,
            created_at: crate::workflows::now_ts(),
        };

        sandboxes()?
            .lock()
            .map_err(|_| "Sandbox store poisoned".to_string())?
            .insert(sandbox_id.clone(), record.clone());

        return Ok(record);
    }
    let builder = docker_builder().await?;
    if config.pull_image {
        builder
            .pull_image(&config.image, None)
            .await
            .map_err(|err| format!("Failed to pull image {}: {err}", config.image))?;
    }

    let sandbox_id = next_sandbox_id();
    let token = token_from_request(request.sidecar_token.as_str());
    let container_name = format!("sidecar-{}", sandbox_id);

    let mut env_vars = Vec::new();
    env_vars.push(format!("SIDECAR_PORT={}", config.container_port));
    env_vars.push(format!("SIDECAR_AUTH_TOKEN={}", token));

    if !request.env_json.trim().is_empty() {
        let env_map = parse_json_object(&request.env_json, "env_json")?;
        if let Some(Value::Object(map)) = env_map {
            for (key, value) in map {
                let val = match value {
                    Value::String(value) => value,
                    Value::Number(value) => value.to_string(),
                    Value::Bool(value) => value.to_string(),
                    _ => continue,
                };
                env_vars.push(format!("{}={}", key, val));
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

    let override_config = BollardConfig {
        exposed_ports: Some(exposed_ports),
        host_config: Some(HostConfig {
            port_bindings: Some(port_bindings),
            ..Default::default()
        }),
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
        .map_err(|err| format!("Failed to start sidecar container: {err}"))?;

    let container_id = container
        .id()
        .ok_or_else(|| "Missing container id".to_string())?
        .to_string();

    let inspect = builder
        .client()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|err| format!("Failed to inspect container: {err}"))?;

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
    };

    sandboxes()?
        .lock()
        .map_err(|_| "Sandbox store poisoned".to_string())?
        .insert(sandbox_id.clone(), record.clone());

    Ok(record)
}

pub async fn stop_sidecar(record: &SandboxRecord) -> Result<(), String> {
    if record.container_id == "mock" {
        return Ok(());
    }
    let builder = docker_builder().await?;
    let mut container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| format!("Failed to load container: {err}"))?;
    container
        .stop()
        .await
        .map_err(|err| format!("Failed to stop container: {err}"))?;
    Ok(())
}

pub async fn resume_sidecar(record: &SandboxRecord) -> Result<(), String> {
    if record.container_id == "mock" {
        return Ok(());
    }
    let builder = docker_builder().await?;
    let mut container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| format!("Failed to load container: {err}"))?;
    container
        .start(false)
        .await
        .map_err(|err| format!("Failed to start container: {err}"))?;
    Ok(())
}

pub async fn delete_sidecar(record: &SandboxRecord) -> Result<(), String> {
    if record.container_id == "mock" {
        return Ok(());
    }
    let builder = docker_builder().await?;
    let container = Container::from_id(builder.client(), &record.container_id)
        .await
        .map_err(|err| format!("Failed to load container: {err}"))?;
    container
        .remove(Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        }))
        .await
        .map_err(|err| format!("Failed to remove container: {err}"))?;
    Ok(())
}

fn extract_ports(
    inspect: &docktopus::bollard::models::ContainerInspectResponse,
    container_port: u16,
    ssh_enabled: bool,
) -> Result<(u16, Option<u16>), String> {
    let network = inspect
        .network_settings
        .as_ref()
        .and_then(|settings| settings.ports.as_ref())
        .ok_or_else(|| "Missing container port mappings".to_string())?;

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
) -> Result<u16, String> {
    let key = format!("{}/tcp", container_port);
    let bindings = ports
        .get(&key)
        .and_then(|value| value.as_ref())
        .ok_or_else(|| format!("Missing port bindings for {key}"))?;
    let host_port = bindings
        .first()
        .and_then(|binding| binding.host_port.as_ref())
        .ok_or_else(|| format!("Missing host port for {key}"))?;
    host_port
        .parse::<u16>()
        .map_err(|_| format!("Invalid host port for {key}"))
}
