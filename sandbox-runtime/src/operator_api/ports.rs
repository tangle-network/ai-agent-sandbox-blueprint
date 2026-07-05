//! Extracted from operator_api.rs — ports route group.

use super::*;

// ---------------------------------------------------------------------------
// Port proxy endpoints
// ---------------------------------------------------------------------------

/// Timeout for proxied user-port requests.
pub(crate) const PORT_PROXY_TIMEOUT: Duration = Duration::from_secs(30);

/// List exposed port mappings for a sandbox.
pub(crate) async fn sandbox_ports_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(json!({ "ports": record.extra_ports })),
    ))
}

/// List exposed port mappings for the singleton instance sandbox.
pub(crate) async fn instance_ports_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::OK,
        Json(json!({ "ports": record.extra_ports })),
    ))
}

/// Reverse-proxy an HTTP request to an exposed container port (with path).
pub(crate) async fn sandbox_port_proxy_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(String, u16, String)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    enforce_session_fanout(&address)?;
    let (sandbox_id, port, path) = params;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    run_port_proxy(record, port, &path, query.as_deref(), method, headers, body).await
}

/// Reverse-proxy to container port root (no sub-path).
pub(crate) async fn sandbox_port_proxy_root_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(String, u16)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    enforce_session_fanout(&address)?;
    let (sandbox_id, port) = params;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    run_port_proxy(record, port, "", query.as_deref(), method, headers, body).await
}

/// Reverse-proxy for instance mode (with path).
pub(crate) async fn instance_port_proxy_handler(
    SessionAuth(address): SessionAuth,
    Path(params): Path<(u16, String)>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    enforce_session_fanout(&address)?;
    let (port, path) = params;
    let record = resolve_instance(&address)?;
    run_port_proxy(record, port, &path, query.as_deref(), method, headers, body).await
}

/// Reverse-proxy for instance mode root (no sub-path).
pub(crate) async fn instance_port_proxy_root_handler(
    SessionAuth(address): SessionAuth,
    Path(port): Path<u16>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    enforce_session_fanout(&address)?;
    let record = resolve_instance(&address)?;
    run_port_proxy(record, port, "", query.as_deref(), method, headers, body).await
}

/// Core proxy logic shared between sandbox and instance handlers.
///
/// Target is always `http://127.0.0.1:{host_port}` — the container port is
/// mapped to a random localhost port by Docker, so SSRF to external hosts is
/// impossible by construction.
pub(crate) async fn run_port_proxy(
    record: SandboxRecord,
    port: u16,
    path: &str,
    query: Option<&str>,
    method: axum::http::Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    // Defense-in-depth: reject clearly malicious path patterns even though the
    // target is always localhost and reqwest::Url::parse validates the result.
    if path.contains('\0') || path.starts_with("//") {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Invalid proxy path".to_string(),
        ));
    }

    circuit_breaker::check_health(&record.id).map_err(circuit_breaker_api_error)?;

    tracing::debug!(
        sandbox_id = %record.id,
        container_port = port,
        method = %method,
        path,
        "port proxy request"
    );

    let build_target =
        |current: &SandboxRecord| -> Result<reqwest::Url, (StatusCode, Json<ApiError>)> {
            let host_port = current.extra_ports.get(&port).copied().ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("Port {port} is not exposed on this sandbox"),
                )
            })?;

            let mut target = format!("http://127.0.0.1:{host_port}/{path}");
            if let Some(qs) = query {
                target.push('?');
                target.push_str(qs);
            }
            reqwest::Url::parse(&target)
                .map_err(|e| api_error(StatusCode::BAD_REQUEST, format!("Invalid path: {e}")))
        };

    let proxy_once = |target_url: reqwest::Url,
                      method: axum::http::Method,
                      headers: HeaderMap,
                      body: axum::body::Bytes| async move {
        match tokio::time::timeout(
            PORT_PROXY_TIMEOUT,
            crate::http::proxy_http(target_url, method, &headers, body.to_vec()),
        )
        .await
        {
            Err(_) => Err(SidecarAttemptFailure::Timeout),
            Ok(Err(err)) => Err(SidecarAttemptFailure::Error(err)),
            Ok(Ok(resp)) => Ok(resp),
        }
    };

    match proxy_once(
        build_target(&record)?,
        method.clone(),
        headers.clone(),
        body.clone(),
    )
    .await
    {
        Err(SidecarAttemptFailure::Timeout) => {
            circuit_breaker::mark_unhealthy(&record.id);
            Err(api_error(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "Port proxy timed out after {}s",
                    PORT_PROXY_TIMEOUT.as_secs()
                ),
            ))
        }
        Err(SidecarAttemptFailure::Error(err)) => {
            if is_retryable_transport_error(&err)
                && let Some(refreshed) = try_refresh_stale_endpoint(&record, "port_proxy").await
            {
                match proxy_once(build_target(&refreshed)?, method, headers, body).await {
                    Ok((status, resp_headers, resp_body)) => {
                        circuit_breaker::mark_healthy(&record.id);
                        runtime::touch_sandbox(&record.id);

                        let mut response =
                            axum::response::Response::builder().status(status.as_u16());
                        for (name, value) in resp_headers.iter() {
                            response = response.header(name.as_str(), value.as_bytes());
                        }

                        return response
                            .body(axum::body::Body::from(resp_body))
                            .map_err(|e| {
                                api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                            });
                    }
                    Err(SidecarAttemptFailure::Timeout) => {
                        circuit_breaker::mark_unhealthy(&record.id);
                        return Err(api_error(
                            StatusCode::GATEWAY_TIMEOUT,
                            format!(
                                "Port proxy timed out after {}s",
                                PORT_PROXY_TIMEOUT.as_secs()
                            ),
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
        Ok((status, resp_headers, resp_body)) => {
            circuit_breaker::mark_healthy(&record.id);
            runtime::touch_sandbox(&record.id);

            let mut response = axum::response::Response::builder().status(status.as_u16());
            for (name, value) in resp_headers.iter() {
                response = response.header(name.as_str(), value.as_bytes());
            }

            response
                .body(axum::body::Body::from(resp_body))
                .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}
