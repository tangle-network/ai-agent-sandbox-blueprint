use std::collections::HashMap;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Result, SandboxError};

const DEFAULT_HOST_AGENT_NETWORK: &str = "bridge";
const DEFAULT_PIDS_LIMIT: u64 = 512;
const DEFAULT_MEMORY_MB: u64 = 512;
const DEFAULT_DISK_MB: u64 = 10 * 1024;
const ENV_SIDECAR_AUTH_DISABLED: &str = "FIRECRACKER_SIDECAR_AUTH_DISABLED";
const ENV_SIDECAR_AUTH_TOKEN: &str = "FIRECRACKER_SIDECAR_AUTH_TOKEN";

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

        let sidecar_auth_disabled = parse_bool_env(ENV_SIDECAR_AUTH_DISABLED, false)?;

        let sidecar_auth_token = std::env::var(ENV_SIDECAR_AUTH_TOKEN).ok().and_then(|v| {
            let trimmed = v.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

        match (sidecar_auth_disabled, sidecar_auth_token.is_some()) {
            (true, true) => {
                return Err(SandboxError::Validation(format!(
                    "{ENV_SIDECAR_AUTH_DISABLED}=true cannot be combined with {ENV_SIDECAR_AUTH_TOKEN}"
                )));
            }
            (false, false) => {
                return Err(SandboxError::Validation(format!(
                    "firecracker requires explicit sidecar auth mode: set {ENV_SIDECAR_AUTH_DISABLED}=true to disable sidecar auth, or set {ENV_SIDECAR_AUTH_TOKEN} when {ENV_SIDECAR_AUTH_DISABLED}=false"
                )));
            }
            _ => {}
        }

        Ok(Self {
            base_url,
            api_key,
            network,
            pids_limit,
            sidecar_auth_token,
        })
    }
}

fn parse_bool_env(name: &str, default: bool) -> Result<bool> {
    let Some(raw) = std::env::var(name).ok() else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(SandboxError::Validation(format!(
            "{name} must be a boolean (true/false/1/0/yes/no/on/off), got '{raw}'"
        ))),
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
    /// User-requested port mappings parsed from `metadata_json.ports`.
    /// Persisted on the sandbox record; **not** forwarded to host-agent yet
    /// because the host-agent OpenAPI in this repo does not specify a
    /// port-forwarding field. See `build_create_payload` for the precise
    /// gap, which is gated for a follow-up once host-agent ships the
    /// contract.
    ///
    /// `#[allow(dead_code)]` is intentional: this field is the seam where
    /// the future host-agent port-forwarding contract will plug in. Read
    /// in `build_create_payload`'s contract assertion test.
    #[allow(dead_code)]
    pub ports: Vec<crate::runtime::PortMapping>,
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

/// Build the JSON body sent to host-agent's `POST /v1/containers`.
///
/// Extracted from `create_and_start` so it can be unit-tested without a
/// running host-agent. The payload shape mirrors fields the host-agent's
/// existing contract already accepts (`sessionId`, `image`, `env`, `labels`,
/// `resources`, `volumes`, `network`, `security`).
///
/// **Port forwarding gap.** The host-agent OpenAPI bundled in this repo
/// (see `dependencies/` and the mock in
/// `sandbox-runtime/tests/firecracker_host_agent.rs`) does not specify a
/// field for inbound port forwarding. `req.ports` is therefore parsed and
/// persisted on the sandbox record (via `metadata_json` round-trip) but
/// **deliberately not** placed in the outbound payload — to do otherwise
/// would invent an upstream contract. Once host-agent publishes a stable
/// port-forwarding field (e.g. `ports: [{containerPort, hostPort, protocol}]`),
/// this function is the single place to add the forwarding.
pub(crate) fn build_create_payload(
    req: &FirecrackerCreateRequest,
    config: &FirecrackerHostAgentConfig,
) -> Value {
    let disk_mb = req.disk_gb.saturating_mul(1024).max(DEFAULT_DISK_MB);
    let memory_mb = req.memory_mb.max(DEFAULT_MEMORY_MB);
    let cpu_cores = req.cpu_cores.max(1);

    json!({
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
    })
}

pub(crate) async fn create_and_start(
    req: FirecrackerCreateRequest,
) -> Result<FirecrackerProvisionResult> {
    let config = FirecrackerHostAgentConfig::load()?;
    let create_payload = build_create_payload(&req, &config);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn set_or_unset(key: &str, value: Option<&str>) {
        match value {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn load_requires_explicit_sidecar_auth_mode() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_or_unset("FIRECRACKER_HOST_AGENT_URL", Some("http://127.0.0.1:18080"));
        set_or_unset(ENV_SIDECAR_AUTH_DISABLED, None);
        set_or_unset(ENV_SIDECAR_AUTH_TOKEN, None);

        let err = FirecrackerHostAgentConfig::load().expect_err("expected validation failure");
        let msg = err.to_string();
        assert!(msg.contains("explicit sidecar auth mode"), "{msg}");
    }

    #[test]
    fn load_rejects_disabled_plus_token() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_or_unset("FIRECRACKER_HOST_AGENT_URL", Some("http://127.0.0.1:18080"));
        set_or_unset(ENV_SIDECAR_AUTH_DISABLED, Some("true"));
        set_or_unset(ENV_SIDECAR_AUTH_TOKEN, Some("secret-token"));

        let err = FirecrackerHostAgentConfig::load().expect_err("expected validation failure");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot be combined"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn load_accepts_disabled_mode() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_or_unset("FIRECRACKER_HOST_AGENT_URL", Some("http://127.0.0.1:18080"));
        set_or_unset(ENV_SIDECAR_AUTH_DISABLED, Some("true"));
        set_or_unset(ENV_SIDECAR_AUTH_TOKEN, None);

        let cfg = FirecrackerHostAgentConfig::load().expect("disabled mode should be valid");
        assert!(cfg.sidecar_auth_token.is_none());
    }

    #[test]
    fn load_accepts_token_mode() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_or_unset("FIRECRACKER_HOST_AGENT_URL", Some("http://127.0.0.1:18080"));
        set_or_unset(ENV_SIDECAR_AUTH_DISABLED, Some("false"));
        set_or_unset(ENV_SIDECAR_AUTH_TOKEN, Some("secret-token"));

        let cfg = FirecrackerHostAgentConfig::load().expect("token mode should be valid");
        assert_eq!(cfg.sidecar_auth_token.as_deref(), Some("secret-token"));
    }

    fn test_config() -> FirecrackerHostAgentConfig {
        FirecrackerHostAgentConfig {
            base_url: "http://127.0.0.1:18080".into(),
            api_key: None,
            network: "bridge".into(),
            pids_limit: DEFAULT_PIDS_LIMIT,
            sidecar_auth_token: None,
        }
    }

    fn req_with_ports(ports: Vec<crate::runtime::PortMapping>) -> FirecrackerCreateRequest {
        FirecrackerCreateRequest {
            session_id: "sess-1".into(),
            image: "ghcr.io/test:latest".into(),
            env: HashMap::new(),
            labels: HashMap::new(),
            cpu_cores: 2,
            memory_mb: 1024,
            disk_gb: 4,
            ports,
        }
    }

    #[test]
    fn build_create_payload_contains_required_fields() {
        let cfg = test_config();
        let body = build_create_payload(&req_with_ports(Vec::new()), &cfg);
        // Required keys established by the host-agent contract present in
        // sandbox-runtime/tests/firecracker_host_agent.rs.
        assert_eq!(body["sessionId"], "sess-1");
        assert_eq!(body["image"], "ghcr.io/test:latest");
        assert_eq!(body["network"], "bridge");
        assert_eq!(body["resources"]["cpu"], 2);
        assert_eq!(body["resources"]["memory"], 1024);
        // disk: 4 GB → 4096 MB, but enforced ≥ 10 GB default ceiling.
        assert!(body["resources"]["disk"].as_u64().unwrap() >= 4096);
        assert_eq!(body["security"]["noNewPrivileges"], true);
    }

    #[test]
    fn build_create_payload_omits_ports_field_until_host_agent_ships_contract() {
        // Regression: until the upstream host-agent OpenAPI specifies a
        // port-forwarding key, we MUST NOT invent one in the outbound
        // payload. The orchestrator parses + persists ports via the
        // metadata_json round-trip; this test pins that the create body
        // stays minimal.
        let cfg = test_config();
        let mapping = crate::runtime::PortMapping {
            container_port: 3000,
            host_port: 30000,
            protocol: crate::runtime::PortProtocol::Tcp,
        };
        let body = build_create_payload(&req_with_ports(vec![mapping]), &cfg);
        assert!(
            body.get("ports").is_none(),
            "create payload must not include `ports` until host-agent contract exists; got {body}"
        );
        assert!(
            body.get("portMappings").is_none(),
            "create payload must not include `portMappings` either; got {body}"
        );
        assert!(
            body.get("port_mappings").is_none(),
            "create payload must not include `port_mappings` either; got {body}"
        );
    }
}
