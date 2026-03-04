use std::collections::HashMap;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Result, SandboxError};

const DEFAULT_HOST_AGENT_NETWORK: &str = "bridge";
const DEFAULT_PIDS_LIMIT: u64 = 512;
const DEFAULT_MEMORY_MB: u64 = 512;
const DEFAULT_DISK_MB: u64 = 10 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct FirecrackerHostAgentConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub network: String,
    pub pids_limit: u64,
    pub sidecar_auth_token: Option<String>,
}

impl FirecrackerHostAgentConfig {
    pub fn load() -> Result<Self> {
        let base_url = std::env::var("FIRECRACKER_HOST_AGENT_URL")
            .or_else(|_| std::env::var("HOST_AGENT_URL"))
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().trim_end_matches('/').to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
            .ok_or_else(|| {
                SandboxError::Validation(
                    "runtime_backend=firecracker requires FIRECRACKER_HOST_AGENT_URL (or HOST_AGENT_URL)".into(),
                )
            })?;

        let api_key = std::env::var("FIRECRACKER_HOST_AGENT_API_KEY")
            .or_else(|_| std::env::var("HOST_AGENT_API_KEY"))
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });

        let network = std::env::var("FIRECRACKER_HOST_AGENT_NETWORK")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
            .unwrap_or_else(|| DEFAULT_HOST_AGENT_NETWORK.to_string());

        let pids_limit = std::env::var("FIRECRACKER_HOST_AGENT_PIDS_LIMIT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_PIDS_LIMIT);

        let sidecar_auth_token = std::env::var("FIRECRACKER_SIDECAR_AUTH_TOKEN")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });

        Ok(Self {
            base_url,
            api_key,
            network,
            pids_limit,
            sidecar_auth_token,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FirecrackerCreateRequest {
    pub session_id: String,
    pub image: String,
    pub env: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub cpu_cores: u64,
    pub memory_mb: u64,
    pub disk_gb: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct FirecrackerContainer {
    pub id: String,
    pub endpoint: Option<String>,
    pub status: Option<String>,
    pub state: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirecrackerContainerStatus {
    Running,
    Stopped,
    Missing,
}

#[derive(Clone, Debug)]
pub(crate) struct FirecrackerProvisionResult {
    pub container: FirecrackerContainer,
    pub sidecar_auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HostAgentContainerResponse {
    id: String,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    state: String,
}

#[derive(Debug, Deserialize)]
struct HostAgentErrorResponse {
    #[serde(default)]
    error: String,
    #[serde(default)]
    code: String,
}

pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    let config = FirecrackerHostAgentConfig::load()?;
    let disk_mb = req.disk_gb.saturating_mul(1024).max(DEFAULT_DISK_MB);
    let memory_mb = req.memory_mb.max(DEFAULT_MEMORY_MB);
    let cpu_cores = req.cpu_cores.max(1);

    let create_payload = json!({
        "sessionId": req.session_id,
        "image": req.image,
        "env": req.env,
        "labels": req.labels,
        "resources": {
            "cpu": cpu_cores,
            "memory": memory_mb,
            "disk": disk_mb,
            "pids": config.pids_limit,
        },
        "volumes": [],
        "network": config.network,
        "security": {
            "readOnly": false,
            "noNewPrivileges": true,
            "user": "0:0",
            "capabilities": {
                "drop": ["ALL"],
                "add": [],
            }
        },
    });

    let created = request_container(
        &config,
        Method::POST,
        "/v1/containers",
        Some(create_payload),
    )
    .await?;

    let mut started = request_container(
        &config,
        Method::POST,
        &format!("/v1/containers/{}/start", created.id),
        None,
    )
    .await?;

    if started.endpoint.is_none() {
        started = request_container(
            &config,
            Method::GET,
            &format!("/v1/containers/{}", created.id),
            None,
        )
        .await?;
    }

    Ok(FirecrackerProvisionResult {
        container: started,
        sidecar_auth_token: config.sidecar_auth_token,
    })
}

pub(crate) async fn start(container_id: &str) -> Result<FirecrackerContainer> {
    let config = FirecrackerHostAgentConfig::load()?;
    let mut started = request_container(
        &config,
        Method::POST,
        &format!("/v1/containers/{container_id}/start"),
        None,
    )
    .await?;
    if started.endpoint.is_none() {
        started = request_container(
            &config,
            Method::GET,
            &format!("/v1/containers/{container_id}"),
            None,
        )
        .await?;
    }
    Ok(started)
}

pub(crate) async fn stop(container_id: &str) -> Result<()> {
    let config = FirecrackerHostAgentConfig::load()?;
    let path = format!("/v1/containers/{container_id}/stop");
    match request_json(&config, Method::POST, &path, None).await {
        Ok(_) => Ok(()),
        Err(SandboxError::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

pub(crate) async fn delete(container_id: &str) -> Result<()> {
    let config = FirecrackerHostAgentConfig::load()?;
    let path = format!("/v1/containers/{container_id}?force=true&removeVolumes=true");
    match request_json(&config, Method::DELETE, &path, None).await {
        Ok(_) => Ok(()),
        Err(SandboxError::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

pub(crate) async fn status(container_id: &str) -> Result<FirecrackerContainerStatus> {
    let config = FirecrackerHostAgentConfig::load()?;
    match request_container(
        &config,
        Method::GET,
        &format!("/v1/containers/{container_id}"),
        None,
    )
    .await
    {
        Ok(container) => {
            let state = container
                .state
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let status = container
                .status
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if state == "running" || status == "running" {
                Ok(FirecrackerContainerStatus::Running)
            } else {
                Ok(FirecrackerContainerStatus::Stopped)
            }
        }
        Err(SandboxError::NotFound(_)) => Ok(FirecrackerContainerStatus::Missing),
        Err(err) => Err(err),
    }
}

pub(crate) async fn health() -> Result<()> {
    let config = FirecrackerHostAgentConfig::load()?;
    let _ = request_json(&config, Method::GET, "/v1/health", None).await?;
    Ok(())
}

fn container_from_value(value: Value) -> Result<FirecrackerContainer> {
    let parsed: HostAgentContainerResponse = serde_json::from_value(value)
        .map_err(|e| SandboxError::Unavailable(format!("invalid host-agent response body: {e}")))?;

    Ok(FirecrackerContainer {
        id: parsed.id,
        endpoint: if parsed.endpoint.trim().is_empty() {
            None
        } else {
            Some(parsed.endpoint)
        },
        status: if parsed.status.trim().is_empty() {
            None
        } else {
            Some(parsed.status)
        },
        state: if parsed.state.trim().is_empty() {
            None
        } else {
            Some(parsed.state)
        },
    })
}

async fn request_container(
    config: &FirecrackerHostAgentConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<FirecrackerContainer> {
    let value = request_json(config, method, path, body).await?;
    container_from_value(value)
}

async fn request_json(
    config: &FirecrackerHostAgentConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let client = crate::util::http_client()?;
    let url = format!(
        "{}/{}",
        config.base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );

    let mut req = client.request(method, &url);
    if let Some(api_key) = &config.api_key {
        req = req.header("x-api-key", api_key);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }

    let resp = req.send().await.map_err(|e| {
        SandboxError::Unavailable(format!(
            "firecracker host-agent request failed for {path}: {e}"
        ))
    })?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        SandboxError::Unavailable(format!(
            "firecracker host-agent body read failed for {path}: {e}"
        ))
    })?;

    if !status.is_success() {
        return Err(map_error_status(status, &text, path));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }

    serde_json::from_str(&text).map_err(|e| {
        SandboxError::Unavailable(format!(
            "firecracker host-agent returned invalid json for {path}: {e}"
        ))
    })
}

fn map_error_status(status: StatusCode, body: &str, path: &str) -> SandboxError {
    let message = parse_host_agent_error(body);
    match status {
        StatusCode::NOT_FOUND => SandboxError::NotFound(format!("{path}: {message}")),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            SandboxError::Validation(format!("{path}: {message}"))
        }
        StatusCode::TOO_MANY_REQUESTS
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::BAD_GATEWAY
        | StatusCode::GATEWAY_TIMEOUT => SandboxError::Unavailable(format!("{path}: {message}")),
        _ => SandboxError::Unavailable(format!(
            "{path}: host-agent status {}: {message}",
            status.as_u16()
        )),
    }
}

fn parse_host_agent_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty error body".to_string();
    }

    if let Ok(parsed) = serde_json::from_str::<HostAgentErrorResponse>(trimmed) {
        if !parsed.error.trim().is_empty() {
            if parsed.code.trim().is_empty() {
                return parsed.error;
            }
            return format!("{} ({})", parsed.error, parsed.code);
        }
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            return error.to_string();
        }
    }

    trimmed.to_string()
}
