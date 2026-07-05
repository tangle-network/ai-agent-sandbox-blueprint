//! Extracted from operator_api.rs — middleware route group.

use super::*;

// ---------------------------------------------------------------------------
// Request ID middleware
// ---------------------------------------------------------------------------

/// Monotonic counter for generating unique request IDs.
pub(crate) static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique identifier attached to every request for correlation in logs and
/// response headers.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

tokio::task_local! {
    /// The request ID for the current task, set by [`request_id_middleware`].
    ///
    /// Downstream helpers (e.g. [`crate::http::sidecar_post_json`]) read this
    /// via `try_with` to propagate the `x-request-id` header to sidecar HTTP
    /// calls, enabling end-to-end trace correlation between operator and
    /// sidecar logs.
    pub(crate) static CURRENT_REQUEST_ID: String;
}

/// Middleware that assigns a unique `x-request-id` to every request.
///
/// The ID is inserted into request extensions (so handlers can access it via
/// `Extension<RequestId>`) and echoed back in the `x-request-id` response
/// header for client-side correlation.  It is also stored in the
/// [`CURRENT_REQUEST_ID`] task-local so that downstream sidecar HTTP calls
/// automatically propagate the same ID.
pub(crate) async fn request_id_middleware(
    mut req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    let id = format!(
        "req-{:016x}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    tracing::debug!(request_id = %id, method = %req.method(), uri = %req.uri(), "incoming request");
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut res = CURRENT_REQUEST_ID.scope(id.clone(), next.run(req)).await;
    res.headers_mut()
        .insert("x-request-id", id.parse().unwrap());
    res
}

// ---------------------------------------------------------------------------
// Security headers middleware
// ---------------------------------------------------------------------------

/// Middleware that adds security headers to every response.
///
/// Applied headers:
/// - `X-Content-Type-Options: nosniff` — prevent MIME-type sniffing
/// - `X-Frame-Options: DENY` — disallow framing (clickjacking protection)
/// - `Cache-Control: no-store` — prevent caching of API responses
pub(crate) async fn security_headers_middleware(
    req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("cache-control", "no-store".parse().unwrap());
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    res
}

// ---------------------------------------------------------------------------
// Auth middleware helper (legacy — prefer `SessionAuth` extractor)
// ---------------------------------------------------------------------------

/// Validate the Authorization header and return the session claims.
///
/// **Prefer** using the [`SessionAuth`](crate::session_auth::SessionAuth) Axum
/// extractor directly in handler signatures instead of calling this manually.
pub fn extract_session_from_headers(
    headers: &HeaderMap,
) -> Result<session_auth::SessionClaims, (StatusCode, Json<ApiError>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = session_auth::extract_bearer_token(auth_header).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid Authorization header format",
        )
    })?;

    session_auth::validate_session_token(token)
        .map_err(|e| api_error(StatusCode::UNAUTHORIZED, e.to_string()))
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

/// Build CORS layer from `CORS_ALLOWED_ORIGINS` env var.
///
/// - `"none"` → CORS disabled (use when behind BPM proxy that handles CORS).
/// - Comma-separated origins → strict whitelist with credentials.
/// - `"*"` → allow any origin (development mode only, must be explicit).
/// - Unset → localhost-only with warning (safe default for production).
pub fn build_cors_layer() -> CorsLayer {
    use axum::http::{Method, header};

    let allowed_methods = vec![
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let allowed_headers = vec![header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT];

    let origins_env = std::env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default();

    // Behind BPM proxy: disable CORS entirely (proxy handles it).
    if origins_env.eq_ignore_ascii_case("none") {
        return CorsLayer::new()
            .allow_origin(AllowOrigin::exact(
                "http://localhost".parse().expect("valid origin"),
            ))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers);
    }

    if origins_env == "*" {
        tracing::warn!("CORS_ALLOWED_ORIGINS=* — wildcard CORS enabled (development mode only)");
        CorsLayer::new()
            .allow_origin(AllowOrigin::any())
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
    } else if origins_env.is_empty() {
        // Unset — restrictive default for production safety.
        tracing::warn!(
            "CORS_ALLOWED_ORIGINS not set; defaulting to localhost-only. \
             Set explicitly for production deployments."
        );
        let localhost_origins: Vec<_> = [
            "http://localhost:1338",
            "http://localhost:3000",
            "http://localhost:5173",
            "http://127.0.0.1:1338",
            "http://127.0.0.1:3000",
            "http://127.0.0.1:5173",
        ]
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(localhost_origins))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
            .allow_credentials(true)
    } else {
        let origins: Vec<_> = origins_env
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
            .allow_credentials(true)
    }
}

// ---------------------------------------------------------------------------
// Per-endpoint HTTP metrics middleware
// ---------------------------------------------------------------------------

pub(crate) async fn http_metrics_middleware(
    req: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    // Prefer the route template (e.g. "/api/sandboxes/{sandbox_id}/exec") to avoid
    // high-cardinality metric keys from dynamic path segments like sandbox IDs.
    // When no route matches (404 paths), use a fixed "unmatched" label to prevent
    // unbounded cardinality from scanners probing arbitrary URLs.
    let path = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = response.status();
    let is_server_error = status.is_server_error();
    let is_client_error = status.is_client_error();
    metrics::http_metrics().record(&path, duration_ms, is_server_error, is_client_error);
    response
}
