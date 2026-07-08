//! Extracted from operator_api.rs — chat_stream route group.

use super::*;

/// Build `/agents/run` payload for prompt/task operations.
pub(crate) struct AgentPayloadRequest<'a> {
    pub(crate) message: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) backend_type: &'a str,
    pub(crate) model: &'a str,
    pub(crate) context_json: &'a str,
    pub(crate) timeout_ms: u64,
    pub(crate) max_turns: Option<u64>,
    pub(crate) agent_identifier: &'a str,
}

pub(crate) fn build_agent_payload(request: AgentPayloadRequest<'_>) -> Value {
    let mut payload = Map::new();
    let identifier = if request.agent_identifier.is_empty() {
        "default"
    } else {
        request.agent_identifier
    };
    payload.insert("identifier".into(), json!(identifier));
    payload.insert("message".into(), json!(request.message));

    if !request.session_id.is_empty() {
        payload.insert("sessionId".into(), json!(request.session_id));
    }

    let mut backend = Map::new();
    if !request.backend_type.is_empty() {
        backend.insert("type".into(), json!(request.backend_type));
    }
    if !request.model.is_empty() {
        backend.insert("model".into(), json!(request.model));
    }
    if !backend.is_empty() {
        payload.insert("backend".into(), Value::Object(backend));
    }

    if let Some(turns) = request.max_turns {
        if turns > 0 {
            let mut metadata = Map::new();
            // Extend from context_json FIRST, then insert maxTurns — so
            // user-supplied context cannot override the operator-enforced
            // turn limit.
            if !request.context_json.trim().is_empty()
                && let Ok(Some(Value::Object(mut ctx))) =
                    crate::util::parse_json_object(request.context_json, "context_json")
            {
                // Strip any attempt to override protected keys
                ctx.remove("maxTurns");
                metadata.extend(ctx);
            }
            metadata.insert("maxTurns".into(), json!(turns));
            payload.insert("metadata".into(), Value::Object(metadata));
        }
    } else if !request.context_json.trim().is_empty()
        && let Ok(Some(Value::Object(ctx))) =
            crate::util::parse_json_object(request.context_json, "context_json")
    {
        payload.insert("metadata".into(), Value::Object(ctx));
    }

    if request.timeout_ms > 0 {
        payload.insert("timeout".into(), json!(request.timeout_ms));
    }
    Value::Object(payload)
}

pub(crate) struct AgentStreamRequest<'a> {
    pub(crate) message: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) backend_type: &'a str,
    pub(crate) model: &'a str,
    pub(crate) context_json: &'a str,
    pub(crate) timeout_ms: u64,
    pub(crate) max_turns: Option<u64>,
}

pub(crate) async fn agent_stream_on_sidecar(
    record: &SandboxRecord,
    request: AgentStreamRequest<'_>,
    mut on_event: impl FnMut(&SidecarSseEvent),
) -> Result<AgentStreamOutcome, (StatusCode, Json<ApiError>)> {
    let payload = build_agent_payload(AgentPayloadRequest {
        message: request.message,
        session_id: request.session_id,
        backend_type: request.backend_type,
        model: request.model,
        context_json: request.context_json,
        timeout_ms: resolve_agent_run_timeout_ms(request.timeout_ms, request.max_turns),
        max_turns: request.max_turns,
        agent_identifier: &record.agent_identifier,
    });
    let client = crate::util::http_client_no_timeout().map_err(|err| {
        api_error(
            StatusCode::BAD_GATEWAY,
            format!("Unable to create sidecar stream client: {err}"),
        )
    })?;
    let mut current_record = record.clone();
    let mut last_retry_after_ms = None;

    for attempt in 0..=AGENT_WARMUP_RETRY_DELAYS_MS.len() {
        let url = build_url(&current_record.sidecar_url, "/agents/run/stream").map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Invalid sidecar stream URL: {err}"),
            )
        })?;
        let mut headers = auth_headers(&current_record.token).map_err(|err| {
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Unable to build sidecar auth headers: {err}"),
            )
        })?;

        if let Ok(rid) = CURRENT_REQUEST_ID.try_with(|id| id.clone())
            && let Ok(value) = reqwest::header::HeaderValue::from_str(&rid)
        {
            headers.insert("x-request-id", value);
        }

        let response = client
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                api_error(
                    StatusCode::BAD_GATEWAY,
                    format!("Agent stream request failed: {err}"),
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown stream error".to_string());
            let parsed_body = serde_json::from_str::<Value>(&body).ok();
            let message = parsed_body
                .as_ref()
                .and_then(|value| value.get("error"))
                .and_then(|error| {
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| error.as_str())
                })
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {status}: {body}"));
            let err = api_error(StatusCode::BAD_GATEWAY, message);
            if let Some(translated) =
                translate_missing_agent_factory_error(record, &record.agent_identifier, &err).await
            {
                return Err(translated);
            }
            if !agent_warmup_retryable(&err) {
                return Err(err);
            }

            circuit_breaker::clear(&record.id);
            if let Some(delay_ms) = AGENT_WARMUP_RETRY_DELAYS_MS.get(attempt).copied() {
                tracing::warn!(
                    request_id = ?request_id_for_logs(),
                    sandbox_id = %record.id,
                    sidecar_url = %current_record.sidecar_url,
                    attempt = attempt + 1,
                    retry_delay_ms = delay_ms,
                    error = %err.1.0.error,
                    "agent warmup detected; retrying prompt/task stream"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                current_record = runtime::get_sandbox_by_id(&record.id)
                    .unwrap_or_else(|_| current_record.clone());
                last_retry_after_ms = Some(delay_ms);
                continue;
            }

            tracing::warn!(
                request_id = ?request_id_for_logs(),
                sandbox_id = %record.id,
                sidecar_url = %current_record.sidecar_url,
                attempts = AGENT_WARMUP_RETRY_DELAYS_MS.len() + 1,
                error = %err.1.0.error,
                "agent warmup retries exhausted for streaming run"
            );
            return Err(api_error_with_details(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sandbox agent is still starting up. Please retry shortly.",
                Some(AGENT_WARMUP_ERROR_CODE),
                last_retry_after_ms,
            ));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut accumulated_text = String::new();
        let mut outcome = AgentStreamOutcome::default();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| {
                api_error(
                    StatusCode::BAD_GATEWAY,
                    format!("Agent stream read failed: {err}"),
                )
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(index) = buffer.find("\n\n") {
                let frame = buffer[..index].to_string();
                buffer = buffer[index + 2..].to_string();

                let Some(event) = parse_sse_event(&frame) else {
                    continue;
                };
                match event.event_type.as_str() {
                    "message.part.updated" => {
                        if let Some(part) = event.data.get("part").and_then(normalize_stream_part)
                            && part.get("type").and_then(Value::as_str) == Some("text")
                            && let Some(text) = part.get("text").and_then(Value::as_str)
                        {
                            accumulated_text = text.to_string();
                        }
                        on_event(&event);
                    }
                    "result" => {
                        outcome = parse_agent_stream_result(&event.data);
                    }
                    "error" => {
                        let message = event
                            .data
                            .get("message")
                            .or_else(|| {
                                event
                                    .data
                                    .get("error")
                                    .and_then(|value| value.get("message"))
                            })
                            .and_then(Value::as_str)
                            .unwrap_or("Agent stream failed");
                        return Err(api_error(StatusCode::BAD_GATEWAY, message));
                    }
                    _ => on_event(&event),
                }
            }
        }

        if outcome.response.is_empty() {
            outcome.response = accumulated_text;
        }
        outcome.success = outcome.error.is_empty();
        return Ok(outcome);
    }

    Err(api_error_with_details(
        StatusCode::SERVICE_UNAVAILABLE,
        "Sandbox agent is still starting up. Please retry shortly.",
        Some(AGENT_WARMUP_ERROR_CODE),
        last_retry_after_ms,
    ))
}
