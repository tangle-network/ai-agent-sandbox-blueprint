//! Extracted from operator_api.rs — agents route group.

use super::*;

/// Build `/terminals/commands` payload for exec operations.
pub(crate) fn build_exec_payload(
    command: &str,
    cwd: &str,
    env_json: &str,
    timeout_ms: u64,
) -> Value {
    let mut payload = Map::new();
    payload.insert("command".to_string(), Value::String(command.to_string()));
    if !cwd.is_empty() {
        payload.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    if timeout_ms > 0 {
        payload.insert("timeout".to_string(), json!(timeout_ms));
    }
    if !env_json.trim().is_empty()
        && let Ok(Some(env_map)) = crate::util::parse_json_object(env_json, "env_json")
    {
        payload.insert("env".to_string(), env_map);
    }
    Value::Object(payload)
}

/// Parse exec response from sidecar.
pub(crate) fn parse_exec_response(parsed: &Value) -> ExecApiResponse {
    let result = parsed.get("result");
    ExecApiResponse {
        exit_code: result
            .and_then(|r| r.get("exitCode"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        stdout: result
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: result
            .and_then(|r| r.get("stderr"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

#[cfg(test)]
pub(crate) fn first_nonempty_output_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

#[cfg(test)]
pub(crate) fn strip_terminal_control_sequences(output: &str) -> String {
    let mut cleaned = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }

        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }

        cleaned.push(ch);
    }

    cleaned
}

#[cfg(test)]
pub(crate) fn summarize_exec_failure(exec: &ExecApiResponse) -> String {
    let stderr = strip_terminal_control_sequences(&exec.stderr);
    let stdout = strip_terminal_control_sequences(&exec.stdout);
    first_nonempty_output_line(&stderr)
        .or_else(|| first_nonempty_output_line(&stdout))
        .unwrap_or("command failed")
        .to_string()
}

#[cfg(test)]
pub(crate) fn parse_detected_ssh_username(
    exec: &ExecApiResponse,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    if exec.exit_code != 0 {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            format!(
                "SSH username detection failed (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(exec)
            ),
        ));
    }

    let stdout = strip_terminal_control_sequences(&exec.stdout);
    for line in stdout.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if crate::ssh_validation::validate_ssh_username(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(api_error(
        StatusCode::BAD_GATEWAY,
        "SSH username detection failed: could not find a valid username in command output",
    ))
}

pub(crate) fn format_available_agents(agents: &[AgentDescriptor]) -> String {
    agents
        .iter()
        .map(|agent| agent.identifier.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn invalid_agent_identifier_error(
    agent_identifier: &str,
    agents: &[AgentDescriptor],
) -> (StatusCode, Json<ApiError>) {
    let trimmed = agent_identifier.trim();
    if agents.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            format!(
                "Unknown agent identifier \"{trimmed}\". This sidecar image does not register that agent."
            ),
        );
    }

    api_error(
        StatusCode::BAD_REQUEST,
        format!(
            "Unknown agent identifier \"{trimmed}\". Available agents: {}",
            format_available_agents(agents)
        ),
    )
}

pub(crate) async fn translate_missing_agent_factory_error(
    record: &SandboxRecord,
    agent_identifier: &str,
    err: &(StatusCode, Json<ApiError>),
) -> Option<(StatusCode, Json<ApiError>)> {
    if agent_identifier.trim().is_empty() {
        return None;
    }

    let message = err.1.0.error.as_str();
    if message.contains("No factory registered for agent identifier") {
        // This is a semantic agent-selection error, not a transport failure.
        // Clear the unhealthy mark so a best-effort /agents lookup can enrich
        // the returned error without restoring hot-path prevalidation.
        circuit_breaker::clear(&record.id);
        let agents = match fetch_sidecar_agents(record).await {
            Ok(Some(agents)) => agents,
            Ok(None) | Err(_) => Vec::new(),
        };
        return Some(invalid_agent_identifier_error(agent_identifier, &agents));
    }

    None
}

pub(crate) fn agent_warmup_retryable(err: &(StatusCode, Json<ApiError>)) -> bool {
    let message = err.1.0.error.as_str();
    message.contains("OpenCode server is not responding")
        || message.contains("Failed to create OpenCode session")
}

pub(crate) fn request_id_for_logs() -> Option<String> {
    CURRENT_REQUEST_ID.try_with(Clone::clone).ok()
}

pub(crate) fn agents_endpoint_unsupported(err: &(StatusCode, Json<ApiError>)) -> bool {
    let message = err.1.0.error.as_str();
    message.contains("HTTP 404") || message.contains("HTTP 405") || message.contains("HTTP 501")
}

pub(crate) fn agent_discovery_not_supported_message(message: &str) -> bool {
    message.contains("HTTP 404") || message.contains("HTTP 405") || message.contains("HTTP 501")
}

pub(crate) fn parse_agent_descriptors(
    parsed: Value,
) -> Result<Vec<AgentDescriptor>, (StatusCode, Json<ApiError>)> {
    serde_json::from_value::<SidecarAgentList>(parsed)
        .map(|body| body.agents)
        .map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Invalid sidecar /agents response: {err}"),
            )
        })
}

pub(crate) async fn fetch_sidecar_agents(
    record: &SandboxRecord,
) -> Result<Option<Vec<AgentDescriptor>>, (StatusCode, Json<ApiError>)> {
    let parsed = match sidecar_get_call(record, "/agents", SIDECAR_DEFAULT_TIMEOUT, "agents").await
    {
        Ok(parsed) => parsed,
        Err(err) if agents_endpoint_unsupported(&err) => return Ok(None),
        Err(err) => return Err(err),
    };

    parse_agent_descriptors(parsed).map(Some)
}

pub(crate) async fn list_agents_on_sidecar(
    record: &SandboxRecord,
) -> Result<Vec<AgentDescriptor>, (StatusCode, Json<ApiError>)> {
    match fetch_sidecar_agents(record).await? {
        Some(agents) => Ok(agents),
        None => Err(api_error(
            StatusCode::NOT_IMPLEMENTED,
            "This sidecar image does not expose agent discovery.",
        )),
    }
}

pub(crate) async fn exec_on_sidecar(
    record: &SandboxRecord,
    req: &ExecApiRequest,
) -> Result<ExecApiResponse, (StatusCode, Json<ApiError>)> {
    let payload = build_exec_payload(&req.command, &req.cwd, &req.env_json, req.timeout_ms);
    let parsed = sidecar_call(
        record,
        "/terminals/commands",
        payload,
        SIDECAR_EXEC_TIMEOUT,
        "exec",
        true,
    )
    .await?;
    Ok(parse_exec_response(&parsed))
}

pub(crate) async fn sandbox_agents_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let agents = list_agents_on_sidecar(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(AgentListApiResponse {
            count: agents.len(),
            agents,
        }),
    ))
}

pub(crate) async fn instance_agents_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let agents = list_agents_on_sidecar(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(AgentListApiResponse {
            count: agents.len(),
            agents,
        }),
    ))
}

// ── Exec ─────────────────────────────────────────────────────────────────

pub(crate) async fn sandbox_exec_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = exec_on_sidecar(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_exec_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<ExecApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let resp = exec_on_sidecar(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}
