use super::*;

pub(crate) fn is_retryable_port_mapping_error(err: &SandboxError) -> bool {
    let SandboxError::Docker(msg) = err else {
        return false;
    };

    msg.starts_with("Missing container port mappings")
        || msg.starts_with("Missing port bindings for ")
        || msg.starts_with("Missing host port for ")
        || (msg.starts_with("Host port for ") && msg.ends_with(" is not assigned yet"))
}

pub(crate) async fn retry_port_mapping_lookup_inner<T, F, Fut>(
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

pub(crate) async fn refresh_port_mapping_with_retry(
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

/// Re-inspect a running container to get its current host port mappings.
///
/// After `docker stop` + `docker start`, Docker may assign new random host ports.
/// Returns `(sidecar_url, sidecar_port, ssh_port, extra_ports)`.
pub(crate) async fn refresh_port_mapping(
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

pub(crate) fn extract_ports(
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

pub(crate) fn extract_host_port(
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

/// Wire protocol for a structured port mapping entry. Mirrors the Linux
/// kernel's `IPPROTO_*` choices that are useful for agent-exposed services;
/// ICMP/SCTP/etc are intentionally not supported because the sandbox network
/// model does not route them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

impl PortProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            PortProtocol::Tcp => "tcp",
            PortProtocol::Udp => "udp",
        }
    }
}

/// Structured port mapping entry parsed from the `ports` field on
/// `metadata_json`. Designed to round-trip through the microvm-runtime
/// network layer once it ships in `0.2.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortMapping {
    pub container_port: u16,
    pub host_port: u16,
    pub protocol: PortProtocol,
}

/// Parse the `ports` field on `metadata_json` into a list of structured
/// `PortMapping` entries. Accepts two shapes (both observed in production
/// inputs) and validates them strictly:
///
/// 1. `[3000]` (legacy) — a bare port number. Treated as
///    `{container_port: N, host_port: N, protocol: tcp}`.
/// 2. `[{"container_port": 3000, "host_port": 30000, "protocol": "tcp"}]`
///    (structured).
///
/// Validation rules:
/// - Each port must be in `1..=65535` (zero is reserved as "unassigned").
/// - Protocol must be `tcp` or `udp` (case-insensitive).
/// - No duplicate `host_port` (would collide on the host network namespace).
/// - No duplicate `container_port` within the same protocol.
/// - Output is capped at [`crate::MAX_EXTRA_PORTS`] entries.
///
/// Returns `Ok(vec![])` when the field is absent, null, or an empty array.
/// Returns `Err(Validation)` on malformed entries so misconfigured deploys
/// fail fast rather than silently dropping ports.
pub fn parse_metadata_ports(metadata_json: &Value) -> Result<Vec<PortMapping>> {
    let arr = match metadata_json.get("ports") {
        Some(Value::Array(a)) => a,
        Some(Value::Null) | None => return Ok(Vec::new()),
        Some(other) => {
            return Err(SandboxError::Validation(format!(
                "metadata_json.ports must be an array, got {}",
                value_kind(other)
            )));
        }
    };

    if arr.is_empty() {
        return Ok(Vec::new());
    }

    let mut out: Vec<PortMapping> = Vec::with_capacity(arr.len().min(crate::MAX_EXTRA_PORTS));
    let mut seen_host = std::collections::HashSet::with_capacity(arr.len());
    let mut seen_container = std::collections::HashSet::with_capacity(arr.len());

    for (idx, entry) in arr.iter().enumerate() {
        let mapping = parse_single_port_mapping(idx, entry)?;
        if !seen_host.insert((mapping.host_port, mapping.protocol)) {
            return Err(SandboxError::Validation(format!(
                "metadata_json.ports[{idx}] duplicate host_port {}/{}",
                mapping.host_port,
                mapping.protocol.as_str()
            )));
        }
        if !seen_container.insert((mapping.container_port, mapping.protocol)) {
            return Err(SandboxError::Validation(format!(
                "metadata_json.ports[{idx}] duplicate container_port {}/{}",
                mapping.container_port,
                mapping.protocol.as_str()
            )));
        }
        out.push(mapping);
        if out.len() == crate::MAX_EXTRA_PORTS {
            // Match `parse_extra_ports`: silently truncate beyond the cap.
            break;
        }
    }
    Ok(out)
}

pub(crate) fn parse_single_port_mapping(idx: usize, entry: &Value) -> Result<PortMapping> {
    let bad = |msg: &str| SandboxError::Validation(format!("metadata_json.ports[{idx}]: {msg}"));

    // Bare integer: legacy compatibility with the existing `[3000]` shape.
    if let Some(n) = entry.as_u64() {
        let port = u16::try_from(n).map_err(|_| bad("port out of range, must be 1..=65535"))?;
        if port == 0 {
            return Err(bad("port 0 is reserved"));
        }
        return Ok(PortMapping {
            container_port: port,
            host_port: port,
            protocol: PortProtocol::Tcp,
        });
    }

    let obj = entry.as_object().ok_or_else(|| {
        bad("each entry must be an integer or an object {container_port,host_port,protocol}")
    })?;

    let container_port = parse_port_field(idx, obj, "container_port")?;
    let host_port = parse_port_field(idx, obj, "host_port")?;
    let protocol = match obj.get("protocol") {
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "tcp" => PortProtocol::Tcp,
            "udp" => PortProtocol::Udp,
            other => {
                return Err(bad(&format!(
                    "protocol must be \"tcp\" or \"udp\", got {other:?}"
                )));
            }
        },
        Some(Value::Null) | None => PortProtocol::Tcp,
        Some(other) => {
            return Err(bad(&format!(
                "protocol must be a string, got {}",
                value_kind(other)
            )));
        }
    };

    Ok(PortMapping {
        container_port,
        host_port,
        protocol,
    })
}

pub(crate) fn parse_port_field(idx: usize, obj: &Map<String, Value>, key: &str) -> Result<u16> {
    let raw = obj.get(key).ok_or_else(|| {
        SandboxError::Validation(format!("metadata_json.ports[{idx}]: missing field {key}"))
    })?;
    let n = raw.as_u64().ok_or_else(|| {
        SandboxError::Validation(format!(
            "metadata_json.ports[{idx}].{key} must be an unsigned integer"
        ))
    })?;
    let port = u16::try_from(n).map_err(|_| {
        SandboxError::Validation(format!(
            "metadata_json.ports[{idx}].{key} out of range, must be 1..=65535"
        ))
    })?;
    if port == 0 {
        return Err(SandboxError::Validation(format!(
            "metadata_json.ports[{idx}].{key} is 0 (reserved)"
        )));
    }
    Ok(port)
}

pub(crate) fn value_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Parse extra port mappings from metadata_json and explicit port_mappings field.
///
/// Ports come from two sources, deduplicated and capped at [`MAX_EXTRA_PORTS`]:
/// 1. `metadata_json` field `"ports"` — a JSON array of port numbers
/// 2. `CreateSandboxParams.port_mappings` — explicit list
///
/// Reserved ports (sidecar HTTP, SSH, and well-known system ports < 1) are excluded.
pub(crate) fn parse_extra_ports(metadata_json: &str, explicit: &[u16]) -> Vec<u16> {
    use crate::MAX_EXTRA_PORTS;
    let config = SidecarRuntimeConfig::load();
    let reserved = [config.container_port, config.ssh_port];

    let mut ports: Vec<u16> = Vec::new();

    // From metadata_json.ports
    if let Ok(Some(meta)) = parse_json_object(metadata_json, "metadata_json")
        && let Some(arr) = meta.get("ports").and_then(|v| v.as_array())
    {
        for v in arr {
            if let Some(p) = v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                ports.push(p);
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
pub(crate) fn extract_extra_ports(
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
