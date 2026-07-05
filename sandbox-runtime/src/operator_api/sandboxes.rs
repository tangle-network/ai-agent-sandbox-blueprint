//! Extracted from operator_api.rs — sandboxes route group.

use super::*;

// ---------------------------------------------------------------------------
// Sandbox endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(crate) struct SandboxSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) sidecar_url: String,
    pub(crate) state: String,
    pub(crate) image: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) agent_identifier: String,
    pub(crate) cpu_cores: u64,
    pub(crate) memory_mb: u64,
    pub(crate) disk_gb: u64,
    pub(crate) created_at: u64,
    pub(crate) last_activity_at: u64,
    pub(crate) ssh_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) managing_operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tee_deployment_id: Option<String>,
    /// Extra user-exposed ports: container_port → host_port.
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub(crate) extra_ports: std::collections::HashMap<u16, u16>,
    /// Seconds of inactivity before the sandbox is automatically stopped.
    pub(crate) idle_timeout_seconds: u64,
    /// Maximum lifetime in seconds before the sandbox is hard-deleted.
    pub(crate) max_lifetime_seconds: u64,
    /// Whether the sandbox has AI credentials configured (e.g. ANTHROPIC_API_KEY).
    pub(crate) credentials_available: bool,
    /// Whether the circuit breaker is currently active for this sandbox's sidecar.
    pub(crate) circuit_breaker_active: bool,
    /// Seconds remaining in the circuit breaker cooldown (if active).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) circuit_breaker_remaining_secs: Option<u64>,
    /// Whether a recovery probe is in flight.
    pub(crate) circuit_breaker_probing: bool,
}

impl SandboxSummary {
    fn from_record(r: &SandboxRecord, managing_operator: Option<&str>) -> Self {
        let breaker = circuit_breaker::query_status(&r.id);
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            sidecar_url: r.sidecar_url.clone(),
            state: match r.state {
                SandboxState::Running => "running".into(),
                SandboxState::Stopped => "stopped".into(),
            },
            image: r.original_image.clone(),
            agent_identifier: r.agent_identifier.clone(),
            cpu_cores: r.cpu_cores,
            memory_mb: r.memory_mb,
            disk_gb: r.disk_gb,
            created_at: r.created_at,
            last_activity_at: r.last_activity_at,
            ssh_port: r.ssh_port,
            service_id: r.service_id,
            managing_operator: managing_operator.map(str::to_string),
            tee_deployment_id: r.tee_deployment_id.clone(),
            extra_ports: r.extra_ports.clone(),
            idle_timeout_seconds: r.idle_timeout_seconds,
            max_lifetime_seconds: r.max_lifetime_seconds,
            credentials_available: workflow_runtime_credentials_available(&r.effective_env_json())
                .unwrap_or(false),
            circuit_breaker_active: breaker.active,
            circuit_breaker_remaining_secs: breaker.remaining_secs,
            circuit_breaker_probing: breaker.probing,
        }
    }
}

pub(crate) fn normalize_operator_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() != 42 || !trimmed.starts_with("0x") {
        return None;
    }
    if trimmed.as_bytes()[2..]
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        Some(trimmed.to_ascii_lowercase())
    } else {
        None
    }
}

pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

pub(crate) fn derive_operator_address_from_secret(
    secret: &[u8],
) -> std::result::Result<String, String> {
    use k256::ecdsa::SigningKey;

    let key_bytes: [u8; 32] = secret
        .try_into()
        .map_err(|_| "operator key must be exactly 32 bytes".to_string())?;
    let signing_key = SigningKey::from_bytes((&key_bytes).into())
        .map_err(|err| format!("invalid operator key bytes: {err}"))?;
    let verifying_key = signing_key.verifying_key();
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let hash = keccak256(pubkey_uncompressed);
    Ok(format!("0x{}", hex::encode(&hash[12..])))
}

pub(crate) fn derive_operator_address_from_keystore_uri(
    keystore_uri: &str,
) -> std::result::Result<String, String> {
    use std::fs;
    use std::path::Path;

    let keystore_path = keystore_uri.strip_prefix("file://").unwrap_or(keystore_uri);
    let ecdsa_dir = Path::new(keystore_path).join("Ecdsa");
    let mut entries = fs::read_dir(&ecdsa_dir)
        .map_err(|err| format!("failed to read {}: {err}", ecdsa_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to enumerate {}: {err}", ecdsa_dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let components: Vec<Vec<u8>> = serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
        if let Some(secret) = components.iter().rev().find(|part| part.len() == 32) {
            return derive_operator_address_from_secret(secret);
        }
    }

    Err(format!(
        "no usable ECDSA secret found under {}",
        ecdsa_dir.display()
    ))
}

pub(crate) fn current_managing_operator() -> Option<String> {
    for key in ["MANAGING_OPERATOR_ADDRESS", "OPERATOR_ADDRESS"] {
        if let Ok(value) = std::env::var(key)
            && let Some(address) = normalize_operator_address(&value)
        {
            return Some(address);
        }
    }

    let keystore_uri = std::env::var("KEYSTORE_URI").ok()?;
    match derive_operator_address_from_keystore_uri(&keystore_uri) {
        Ok(address) => Some(address),
        Err(err) => {
            tracing::warn!(error = %err, "Failed to derive managing operator address from keystore");
            None
        }
    }
}

pub(crate) async fn list_sandboxes(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    if let Ok(repaired) = runtime::repair_sandbox_service_links_from_provisions()
        && repaired > 0
    {
        tracing::info!(
            repaired,
            "Repaired missing sandbox service links from provision metadata"
        );
    }

    let managing_operator = current_managing_operator();
    match sandboxes().and_then(|s| s.values()) {
        Ok(records) => {
            let summaries: Vec<SandboxSummary> = records
                .into_iter()
                .filter(|r| !r.owner.is_empty() && r.owner.eq_ignore_ascii_case(&address))
                .filter_map(|mut record| {
                    if let Err(e) = runtime::unseal_record(&mut record) {
                        tracing::warn!(id = %record.id, error = %e, "Failed to unseal record in listing — skipping");
                        return None;
                    }
                    Some(SandboxSummary::from_record(&record, managing_operator.as_deref()))
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "sandboxes": summaries })),
            )
                .into_response()
        }
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}
