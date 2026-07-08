//! Extracted from operator_api.rs — errors route group.

use super::*;

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub(crate) error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_after_ms: Option<u64>,
}

pub(crate) fn api_error(
    status: StatusCode,
    msg: impl Into<String>,
) -> (StatusCode, Json<ApiError>) {
    api_error_with_details(status, msg, None, None)
}

pub(crate) fn api_error_with_details(
    status: StatusCode,
    msg: impl Into<String>,
    code: Option<&str>,
    retry_after_ms: Option<u64>,
) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: msg.into(),
            code: code.map(str::to_string),
            retry_after_ms,
        }),
    )
}

/// Convert a `SandboxError` from `circuit_breaker::check_health` into a
/// structured 503 response with the `CIRCUIT_BREAKER` error code.
pub(crate) fn circuit_breaker_api_error(err: SandboxError) -> (StatusCode, Json<ApiError>) {
    match err {
        SandboxError::CircuitBreaker {
            remaining_secs,
            probing,
        } => api_error_with_details(
            StatusCode::SERVICE_UNAVAILABLE,
            if probing {
                "Sidecar recovery probe in progress. Please retry shortly.".to_string()
            } else {
                format!("Sidecar is in circuit-breaker cooldown ({remaining_secs}s remaining).")
            },
            Some("CIRCUIT_BREAKER"),
            Some(remaining_secs * 1000),
        ),
        other => api_error(StatusCode::SERVICE_UNAVAILABLE, other.to_string()),
    }
}

/// Enforce the per-session fanout limiter for high-cost endpoints (port
/// proxy, chat run/stream). NAT'd users would otherwise share an IP-tier
/// bucket — this caps a single authenticated session's expensive
/// downstream calls regardless of source IP.
pub(crate) fn enforce_session_fanout(
    address: &str,
) -> std::result::Result<(), (StatusCode, Json<ApiError>)> {
    match rate_limit::check_session_fanout(address) {
        Ok(()) => Ok(()),
        Err(retry_after_secs) => Err(api_error_with_details(
            StatusCode::TOO_MANY_REQUESTS,
            "Per-session rate limit exceeded".to_string(),
            Some("SESSION_RATE_LIMIT"),
            Some(retry_after_secs * 1000),
        )),
    }
}

/// Generic 500 for `serde_json::Error` from response-body serialization. The
/// detail goes to logs, not the wire — these are programming errors, not
/// user-facing.
pub(crate) fn json_serialization_error(e: serde_json::Error) -> axum::response::Response {
    tracing::error!(err = %e, "JSON serialization failure");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Response serialization failed".to_string(),
    )
    .into_response()
}

/// Map a `SandboxError` to a typed HTTP response. Use this in handler
/// `Err(_)` arms instead of `api_error(INTERNAL_SERVER_ERROR, e.to_string())`
/// so we don't leak raw error chains, container internals, or RPC error
/// strings to API consumers.
///
/// Variants surface their messages directly when those messages are
/// already user-facing (auth, validation, not-found, unavailable). Internal
/// failures (docker, storage, cloud-provider, http) are logged at `error`
/// level and return a generic message — operators see the detail in
/// observability, callers see only that the request failed.
pub(crate) fn classify_sandbox_error(err: SandboxError) -> (StatusCode, Json<ApiError>) {
    match err {
        SandboxError::Auth(msg) => api_error(StatusCode::UNAUTHORIZED, msg),
        SandboxError::Validation(msg) => api_error(StatusCode::BAD_REQUEST, msg),
        SandboxError::NotFound(msg) => api_error(StatusCode::NOT_FOUND, msg),
        SandboxError::Unavailable(msg) => api_error(StatusCode::SERVICE_UNAVAILABLE, msg),
        // Feature is not yet implemented in the underlying runtime primitive.
        // `501 Not Implemented` is the right shape — the request is well-formed
        // and the caller is authenticated; the server simply has not yet wired
        // the capability. Surface the message so callers learn which release to
        // wait for.
        SandboxError::Unsupported(msg) => api_error(StatusCode::NOT_IMPLEMENTED, msg),
        SandboxError::CircuitBreaker { .. } => circuit_breaker_api_error(err),
        SandboxError::Http(detail) => {
            tracing::error!(err = %detail, "upstream HTTP failure");
            api_error(
                StatusCode::BAD_GATEWAY,
                "Upstream request failed".to_string(),
            )
        }
        SandboxError::CloudProvider(detail) => {
            tracing::error!(err = %detail, "cloud provider failure");
            api_error(
                StatusCode::BAD_GATEWAY,
                "Cloud provider request failed".to_string(),
            )
        }
        SandboxError::Docker(detail) => {
            tracing::error!(err = %detail, "container runtime failure");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Container runtime error".to_string(),
            )
        }
        SandboxError::Storage(detail) => {
            tracing::error!(err = %detail, "storage failure");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Storage error".to_string(),
            )
        }
    }
}
