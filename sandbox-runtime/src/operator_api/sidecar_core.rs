//! Extracted from operator_api.rs — sidecar_core route group.

use super::*;

pub(crate) enum SidecarAttemptFailure {
    Timeout,
    Error(SandboxError),
}

pub(crate) fn is_retryable_transport_error(err: &SandboxError) -> bool {
    matches!(err, SandboxError::Http(msg) if msg.contains("error sending request for url"))
}

pub(crate) fn extract_http_status_code(message: &str) -> Option<u16> {
    let (_, tail) = message.split_once("HTTP ")?;
    tail.split_whitespace().next()?.parse::<u16>().ok()
}

pub(crate) fn terminal_api_error_status(err: &SandboxError) -> Option<u16> {
    match err {
        SandboxError::Http(message) => extract_http_status_code(message),
        _ => None,
    }
}

pub(crate) fn terminal_api_error_status_from_response(
    err: &(StatusCode, Json<ApiError>),
) -> Option<u16> {
    extract_http_status_code(err.1.0.error.as_str())
}

pub(crate) fn terminal_sidecar_error_code(message: &str) -> Option<&'static str> {
    if message.contains("SESSION_NOT_FOUND") {
        return Some("SESSION_NOT_FOUND");
    }
    if message.contains("SESSION_NOT_RUNNING") {
        return Some("SESSION_NOT_RUNNING");
    }
    None
}

pub(crate) fn terminal_api_error_response(
    op_name: &str,
    status: u16,
    message: Option<&str>,
) -> (StatusCode, Json<ApiError>) {
    if op_name == "terminal detail" && status == 404 {
        return api_error(StatusCode::NOT_FOUND, "Terminal session not found");
    }

    if let Some(message) = message {
        match terminal_sidecar_error_code(message) {
            Some("SESSION_NOT_FOUND") => {
                return api_error(StatusCode::NOT_FOUND, "Terminal session not found");
            }
            Some("SESSION_NOT_RUNNING") => {
                return api_error(StatusCode::CONFLICT, "Terminal session is not running");
            }
            _ => {}
        }
    }

    api_error_with_details(
        StatusCode::BAD_GATEWAY,
        "Sidecar PTY terminal API is not supported by this sandbox image/runtime.",
        Some(TERMINAL_UNSUPPORTED_ERROR_CODE),
        None,
    )
}

pub(crate) fn terminal_api_error(
    err: &SandboxError,
    op_name: &str,
) -> Option<(StatusCode, Json<ApiError>)> {
    let status = terminal_api_error_status(err)?;
    if (400..500).contains(&status) || status == 501 {
        let message = match err {
            SandboxError::Http(message) => Some(message.as_str()),
            _ => None,
        };
        return Some(terminal_api_error_response(op_name, status, message));
    }
    None
}

pub(crate) fn terminal_api_error_from_response(
    err: &(StatusCode, Json<ApiError>),
    op_name: &str,
) -> Option<(StatusCode, Json<ApiError>)> {
    let status = terminal_api_error_status_from_response(err)?;
    if (400..500).contains(&status) || status == 501 {
        return Some(terminal_api_error_response(
            op_name,
            status,
            Some(err.1.0.error.as_str()),
        ));
    }
    None
}

pub(crate) async fn try_refresh_stale_endpoint(
    record: &SandboxRecord,
    op_name: &str,
) -> Option<SandboxRecord> {
    if !runtime::supports_docker_endpoint_refresh(record) {
        return None;
    }

    match runtime::refresh_docker_sandbox_endpoint(record).await {
        Ok(updated) => Some(updated),
        Err(err) => {
            tracing::warn!(
                sandbox_id = %record.id,
                operation = op_name,
                error = %err,
                "failed to refresh stale sandbox endpoint"
            );
            None
        }
    }
}

pub(crate) async fn run_sidecar_json_attempt(
    record: &SandboxRecord,
    path: &str,
    payload: &Value,
    timeout: Duration,
) -> std::result::Result<Value, SidecarAttemptFailure> {
    match tokio::time::timeout(
        timeout,
        sidecar_post_json_without_timeout(
            &record.sidecar_url,
            path,
            &record.token,
            payload.clone(),
        ),
    )
    .await
    {
        Err(_) => Err(SidecarAttemptFailure::Timeout),
        Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
        Ok(Ok(parsed)) => Ok(parsed),
    }
}

pub(crate) async fn run_sidecar_get_json_attempt(
    record: &SandboxRecord,
    path: &str,
    timeout: Duration,
) -> std::result::Result<Value, SidecarAttemptFailure> {
    match tokio::time::timeout(
        timeout,
        sidecar_get_json(&record.sidecar_url, path, &record.token),
    )
    .await
    {
        Err(_) => Err(SidecarAttemptFailure::Timeout),
        Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
        Ok(Ok(parsed)) => Ok(parsed),
    }
}

pub(crate) async fn run_sidecar_patch_json_attempt(
    record: &SandboxRecord,
    path: &str,
    payload: &Value,
    timeout: Duration,
) -> std::result::Result<Value, SidecarAttemptFailure> {
    let path = path.to_string();
    let payload = payload.clone();
    let sidecar_url = record.sidecar_url.clone();
    let token = record.token.clone();

    match tokio::time::timeout(timeout, async move {
        let url = build_url(&sidecar_url, &path)?;
        let mut headers = auth_headers(&token)?;
        if let Ok(rid) = CURRENT_REQUEST_ID.try_with(|id| id.clone())
            && let Ok(value) = reqwest::header::HeaderValue::from_str(&rid)
        {
            headers.insert("x-request-id", value);
        }

        let response = crate::util::http_client()?
            .patch(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|err| SandboxError::Http(format!("HTTP request failed: {err}")))?;

        let status = response.status();
        let parsed = response
            .json::<Value>()
            .await
            .map_err(|err| SandboxError::Http(format!("invalid JSON response: {err}")))?;
        if !status.is_success() {
            return Err(SandboxError::Http(format!("HTTP {status}: {parsed}")));
        }

        Ok(parsed)
    })
    .await
    {
        Err(_) => Err(SidecarAttemptFailure::Timeout),
        Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
        Ok(Ok(parsed)) => Ok(parsed),
    }
}
