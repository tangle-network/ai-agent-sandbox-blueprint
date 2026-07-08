//! Extracted from operator_api.rs — health route group.

use super::*;

// ---------------------------------------------------------------------------
// Provision progress endpoints
// ---------------------------------------------------------------------------

pub(crate) async fn get_provision(Path(call_id): Path<u64>) -> impl IntoResponse {
    match provision_progress::get_provision(call_id) {
        Ok(Some(status)) => match serde_json::to_value(status) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => json_serialization_error(e),
        },
        Ok(None) => api_error(StatusCode::NOT_FOUND, "Provision not found").into_response(),
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

pub(crate) async fn list_provisions() -> impl IntoResponse {
    match provision_progress::list_all_provisions() {
        Ok(provisions) => (
            StatusCode::OK,
            Json(serde_json::json!({ "provisions": provisions })),
        )
            .into_response(),
        Err(e) => classify_sandbox_error(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Health & metrics endpoints (unauthenticated)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) enum RuntimeProbeBackend {
    Docker,
    Firecracker,
    Tee,
}

impl RuntimeProbeBackend {
    fn as_str(self) -> &'static str {
        match self {
            RuntimeProbeBackend::Docker => "docker",
            RuntimeProbeBackend::Firecracker => "firecracker",
            RuntimeProbeBackend::Tee => "tee",
        }
    }
}

pub(crate) fn configured_runtime_probe_backend() -> Result<RuntimeProbeBackend, String> {
    let raw = std::env::var("SANDBOX_RUNTIME_BACKEND").unwrap_or_else(|_| "docker".to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "docker" | "container" => Ok(RuntimeProbeBackend::Docker),
        "firecracker" | "microvm" => Ok(RuntimeProbeBackend::Firecracker),
        "tee" | "confidential" | "confidential-vm" => Ok(RuntimeProbeBackend::Tee),
        _ => Err(format!(
            "invalid SANDBOX_RUNTIME_BACKEND '{raw}' (expected docker|firecracker|tee)"
        )),
    }
}

pub(crate) async fn probe_runtime_backend() -> (String, bool, Option<String>) {
    let backend = match configured_runtime_probe_backend() {
        Ok(v) => v,
        Err(err) => return ("invalid".to_string(), false, Some(err)),
    };

    match backend {
        RuntimeProbeBackend::Docker => {
            let ok = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let builder = runtime::docker_builder().await.ok()?;
                builder.client().ping().await.ok()?;
                Some(())
            })
            .await
            .is_ok_and(|v| v.is_some());

            (
                backend.as_str().to_string(),
                ok,
                if ok {
                    None
                } else {
                    Some("docker daemon unreachable".to_string())
                },
            )
        }
        RuntimeProbeBackend::Firecracker => {
            let checked = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::firecracker::health(),
            )
            .await;
            match checked {
                Ok(Ok(())) => (backend.as_str().to_string(), true, None),
                Ok(Err(err)) => (backend.as_str().to_string(), false, Some(err.to_string())),
                Err(_) => (
                    backend.as_str().to_string(),
                    false,
                    Some("firecracker driver health check timed out".to_string()),
                ),
            }
        }
        RuntimeProbeBackend::Tee => {
            let ok = crate::tee::try_tee_backend().is_some();
            (
                backend.as_str().to_string(),
                ok,
                if ok {
                    None
                } else {
                    Some("tee backend not initialized".to_string())
                },
            )
        }
    }
}

pub(crate) async fn health() -> impl IntoResponse {
    let (runtime_backend, runtime_ok, runtime_error) = probe_runtime_backend().await;

    // Check persistent store readability.
    let store_ok = runtime::sandboxes().and_then(|s| s.values()).is_ok();

    let (status, code) = match (runtime_ok, store_ok) {
        (true, true) => ("ok", StatusCode::OK),
        _ => ("degraded", StatusCode::SERVICE_UNAVAILABLE),
    };

    let check = |ok: bool| {
        if ok {
            json!({ "status": "ok" })
        } else {
            json!({ "status": "error" })
        }
    };

    (
        code,
        Json(json!({
            "status": status,
            "checks": {
                "runtime": check(runtime_ok),
                "store": check(store_ok),
            },
            "runtime_backend": runtime_backend,
            "runtime_error": runtime_error,
        })),
    )
}

/// Readiness probe — reports ready only when Docker daemon is reachable
/// AND the persistent store is functional. Returns 503 during startup or
/// when either subsystem is degraded. Kubernetes should route traffic only
/// to ready instances (`readinessProbe` on this endpoint).
pub(crate) async fn readyz() -> impl IntoResponse {
    let (runtime_backend, runtime_ok, runtime_error) = probe_runtime_backend().await;
    let store_ok = runtime::sandboxes().and_then(|s| s.values()).is_ok();

    if runtime_ok && store_ok {
        (StatusCode::OK, Json(json!({ "status": "ready" })))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "runtime_backend": runtime_backend,
                "runtime": runtime_ok,
                "runtime_error": runtime_error,
                "store": store_ok,
            })),
        )
    }
}

pub(crate) async fn prometheus_metrics() -> impl IntoResponse {
    let mut body = metrics::metrics().render_prometheus();
    body.push_str(&metrics::http_metrics().render_prometheus());
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentDescriptor {
    pub(crate) identifier: String,
    #[serde(
        rename = "displayName",
        alias = "display_name",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub(crate) display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SidecarAgentList {
    #[serde(default)]
    pub(crate) agents: Vec<AgentDescriptor>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentListApiResponse {
    pub(crate) agents: Vec<AgentDescriptor>,
    pub(crate) count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeCapabilityDescriptor {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) description: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct HarnessCapabilityDescriptor {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) mcp: bool,
    pub(crate) skills: bool,
    pub(crate) subagents: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeCapabilitiesResponse {
    pub(crate) capabilities: Vec<RuntimeCapabilityDescriptor>,
    pub(crate) harnesses: Vec<HarnessCapabilityDescriptor>,
}

pub(crate) fn runtime_capabilities_response() -> RuntimeCapabilitiesResponse {
    RuntimeCapabilitiesResponse {
        capabilities: vec![
            RuntimeCapabilityDescriptor {
                id: "computer_use",
                label: "Computer Use",
                description: "Enable browser/computer-use sidecar services.",
            },
            RuntimeCapabilityDescriptor {
                id: "all_harness",
                label: "All Harness Runtime",
                description: "Enable the open-source all-harness agent runtime: Claude, Codex, opencode, Kimi, and Gemini.",
            },
        ],
        harnesses: vec![
            HarnessCapabilityDescriptor {
                id: "claude-code",
                label: "Claude Code",
                mcp: true,
                skills: true,
                subagents: true,
            },
            HarnessCapabilityDescriptor {
                id: "codex",
                label: "Codex",
                mcp: true,
                skills: false,
                subagents: false,
            },
            HarnessCapabilityDescriptor {
                id: "opencode",
                label: "opencode",
                mcp: true,
                skills: true,
                subagents: true,
            },
            HarnessCapabilityDescriptor {
                id: "kimi-code",
                label: "Kimi Code",
                mcp: true,
                skills: false,
                subagents: false,
            },
            HarnessCapabilityDescriptor {
                id: "gemini",
                label: "Gemini CLI",
                mcp: true,
                skills: false,
                subagents: false,
            },
        ],
    }
}

pub(crate) async fn capabilities_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(runtime_capabilities_response()))
}
