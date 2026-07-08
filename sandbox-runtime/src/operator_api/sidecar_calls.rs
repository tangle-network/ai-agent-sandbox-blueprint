//! Extracted from operator_api.rs — sidecar_calls route group.

use super::*;

/// Call a sidecar endpoint with circuit-breaker integration and timeout.
///
/// This is the single entry point for all sidecar HTTP calls. It:
/// 1. Checks the circuit breaker (returns 503 if in cooldown)
/// 2. Sends the request with the given timeout
/// 3. Marks the sidecar healthy/unhealthy based on the outcome
/// 4. Touches the sandbox activity timestamp on success
pub(crate) async fn sidecar_call(
    record: &SandboxRecord,
    path: &str,
    payload: Value,
    timeout: Duration,
    op_name: &str,
    allow_transport_retry: bool,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_json_attempt(record, path, &payload, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if allow_transport_retry
                && is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match run_sidecar_json_attempt(&refreshed, path, &payload, timeout).await {
                    Ok(parsed) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(parsed);
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Err(SidecarAttemptFailure::Error(retry_err)) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

pub(crate) async fn terminal_sidecar_call(
    record: &SandboxRecord,
    path: &str,
    payload: Value,
    timeout: Duration,
    op_name: &str,
    allow_transport_retry: bool,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_json_attempt(record, path, &payload, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if let Some(api_err) = terminal_api_error(&err, op_name) {
                return Err(api_err);
            }

            if allow_transport_retry
                && is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match run_sidecar_json_attempt(&refreshed, path, &payload, timeout).await {
                    Ok(parsed) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(parsed);
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Err(SidecarAttemptFailure::Error(retry_err)) => {
                        if let Some(api_err) = terminal_api_error(&retry_err, op_name) {
                            return Err(api_err);
                        }
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

pub(crate) async fn sidecar_get_call(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
    op_name: &str,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_get_json_attempt(record, path, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            let err_message = err.to_string();
            if op_name == "agents" && agent_discovery_not_supported_message(&err_message) {
                return Err(api_error(StatusCode::BAD_GATEWAY, err_message));
            }

            if is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match run_sidecar_get_json_attempt(&refreshed, path, timeout).await {
                    Ok(parsed) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(parsed);
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Err(SidecarAttemptFailure::Error(retry_err)) => {
                        let retry_message = retry_err.to_string();
                        if op_name == "agents"
                            && agent_discovery_not_supported_message(&retry_message)
                        {
                            return Err(api_error(StatusCode::BAD_GATEWAY, retry_message));
                        }
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(StatusCode::BAD_GATEWAY, retry_message));
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err_message))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

pub(crate) async fn terminal_sidecar_get_call(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
    op_name: &str,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_get_json_attempt(record, path, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if let Some(api_err) = terminal_api_error(&err, op_name) {
                return Err(api_err);
            }

            if is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match run_sidecar_get_json_attempt(&refreshed, path, timeout).await {
                    Ok(parsed) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(parsed);
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Err(SidecarAttemptFailure::Error(retry_err)) => {
                        if let Some(api_err) = terminal_api_error(&retry_err, op_name) {
                            return Err(api_err);
                        }
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

pub(crate) async fn terminal_sidecar_patch_call(
    record: &SandboxRecord,
    path: &str,
    payload: Value,
    timeout: Duration,
    op_name: &str,
) -> Result<Value, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match run_sidecar_patch_json_attempt(record, path, &payload, timeout).await {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if let Some(api_err) = terminal_api_error(&err, op_name) {
                return Err(api_err);
            }

            if is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match run_sidecar_patch_json_attempt(&refreshed, path, &payload, timeout).await {
                    Ok(parsed) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(parsed);
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Err(SidecarAttemptFailure::Error(retry_err)) => {
                        if let Some(api_err) = terminal_api_error(&retry_err, op_name) {
                            return Err(api_err);
                        }
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(StatusCode::BAD_GATEWAY, err.to_string()))
        }
        Ok(parsed) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(parsed)
        }
    }
}

pub(crate) async fn open_sidecar_stream_attempt(
    record: &SandboxRecord,
    path: &str,
) -> std::result::Result<reqwest::Response, SidecarAttemptFailure> {
    let url = match build_url(&record.sidecar_url, path) {
        Ok(url) => url,
        Err(err) => return Err(SidecarAttemptFailure::Error(err)),
    };
    let mut headers = match auth_headers(&record.token) {
        Ok(headers) => headers,
        Err(err) => return Err(SidecarAttemptFailure::Error(err)),
    };

    if let Ok(rid) = CURRENT_REQUEST_ID.try_with(|id| id.clone())
        && let Ok(value) = reqwest::header::HeaderValue::from_str(&rid)
    {
        headers.insert("x-request-id", value);
    }

    let client = match crate::util::http_client_no_timeout() {
        Ok(client) => client,
        Err(err) => return Err(SidecarAttemptFailure::Error(err)),
    };

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            SidecarAttemptFailure::Error(SandboxError::Http(format!("HTTP request failed: {err}")))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown stream error".to_string());
        return Err(SidecarAttemptFailure::Error(SandboxError::Http(format!(
            "HTTP {status}: {body}"
        ))));
    }

    Ok(response)
}

pub(crate) async fn terminal_sidecar_stream_call(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
    op_name: &str,
) -> Result<reqwest::Response, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    match tokio::time::timeout(timeout, open_sidecar_stream_attempt(record, path)).await {
        Err(_) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Ok(Err(err)) => {
            if let SidecarAttemptFailure::Error(ref inner) = err {
                if let Some(api_err) = terminal_api_error(inner, op_name) {
                    return Err(api_err);
                }

                if is_retryable_transport_error(inner)
                    && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
                {
                    match tokio::time::timeout(
                        timeout,
                        open_sidecar_stream_attempt(&refreshed, path),
                    )
                    .await
                    {
                        Err(_) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(
                                StatusCode::GATEWAY_TIMEOUT,
                                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                            ));
                        }
                        Ok(Ok(response)) => {
                            circuit_breaker::mark_healthy(&record.id);
                            runtime::touch_sandbox(&record.id);
                            return Ok(response);
                        }
                        Ok(Err(SidecarAttemptFailure::Error(retry_err))) => {
                            if let Some(api_err) = terminal_api_error(&retry_err, op_name) {
                                return Err(api_err);
                            }
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(StatusCode::BAD_GATEWAY, retry_err.to_string()));
                        }
                        Ok(Err(SidecarAttemptFailure::Timeout)) => {
                            circuit_breaker::mark_unhealthy(&record.id);
                            return Err(api_error(
                                StatusCode::GATEWAY_TIMEOUT,
                                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                            ));
                        }
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            match err {
                SidecarAttemptFailure::Timeout => Err(api_error(
                    StatusCode::GATEWAY_TIMEOUT,
                    format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                )),
                SidecarAttemptFailure::Error(inner) => {
                    Err(api_error(StatusCode::BAD_GATEWAY, inner.to_string()))
                }
            }
        }
        Ok(Ok(response)) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(response)
        }
    }
}

pub(crate) async fn terminal_sidecar_delete_call(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
    op_name: &str,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    let path = path.to_string();
    let run_delete = |sidecar_url: String, token: String| {
        let path = path.clone();
        async move {
            let url = build_url(&sidecar_url, &path)
                .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?;
            let mut headers = auth_headers(&token)
                .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?;
            if let Ok(rid) = CURRENT_REQUEST_ID.try_with(|id| id.clone())
                && let Ok(value) = reqwest::header::HeaderValue::from_str(&rid)
            {
                headers.insert("x-request-id", value);
            }

            let client = crate::util::http_client()
                .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?
                .delete(url)
                .headers(headers)
                .send()
                .await
                .map_err(|err| {
                    api_error(
                        StatusCode::BAD_GATEWAY,
                        format!("HTTP request failed: {err}"),
                    )
                })?;
            if !client.status().is_success() {
                let status = client.status();
                let body = client
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown delete error".to_string());
                return Err(api_error(
                    StatusCode::BAD_GATEWAY,
                    format!("HTTP {status}: {body}"),
                ));
            }
            Ok(())
        }
    };

    match tokio::time::timeout(
        timeout,
        run_delete(record.sidecar_url.clone(), record.token.clone()),
    )
    .await
    {
        Err(_) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
            ))
        }
        Ok(Err(err)) => {
            if let Some(api_err) = terminal_api_error_from_response(&err, op_name) {
                return Err(api_err);
            }

            let err_text = err.1.0.error.clone();
            if err_text.contains("error sending request for url")
                && let Some(refreshed) = try_refresh_stale_endpoint(record, op_name).await
            {
                match tokio::time::timeout(
                    timeout,
                    run_delete(refreshed.sidecar_url.clone(), refreshed.token.clone()),
                )
                .await
                {
                    Err(_) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!("Sidecar {op_name} timed out after {}s", timeout.as_secs()),
                        ));
                    }
                    Ok(Ok(())) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);
                        return Ok(());
                    }
                    Ok(Err(retry_err)) => {
                        if let Some(api_err) = terminal_api_error_from_response(&retry_err, op_name)
                        {
                            return Err(api_err);
                        }
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(retry_err);
                    }
                }
            }

            circuit_breaker::mark_unhealthy(&record.id);
            Err(err)
        }
        Ok(Ok(())) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);
            Ok(())
        }
    }
}
