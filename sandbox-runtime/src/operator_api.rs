//! Axum-based operator API for sandbox management.
//!
//! Provides REST endpoints for:
//! - Listing active sandboxes
//! - Querying provision progress
//! - Session auth (challenge/response + PASETO tokens)

use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::metrics;
use crate::provision_progress;
use crate::rate_limit;
use crate::runtime::{SandboxRecord, SandboxState, sandboxes};
use crate::secret_provisioning;
use crate::session_auth::{self, SessionAuth};

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ApiError {
    error: String,
}

pub(crate) fn api_error(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: msg.into() }))
}

// ---------------------------------------------------------------------------
// Sandbox endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SandboxSummary {
    id: String,
    sidecar_url: String,
    state: String,
    cpu_cores: u64,
    memory_mb: u64,
    created_at: u64,
    last_activity_at: u64,
}

impl From<&SandboxRecord> for SandboxSummary {
    fn from(r: &SandboxRecord) -> Self {
        Self {
            id: r.id.clone(),
            sidecar_url: r.sidecar_url.clone(),
            state: match r.state {
                SandboxState::Running => "running".into(),
                SandboxState::Stopped => "stopped".into(),
            },
            cpu_cores: r.cpu_cores,
            memory_mb: r.memory_mb,
            created_at: r.created_at,
            last_activity_at: r.last_activity_at,
        }
    }
}

async fn list_sandboxes(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    match sandboxes().and_then(|s| s.values()) {
        Ok(records) => {
            let summaries: Vec<SandboxSummary> = records
                .iter()
                .filter(|r| r.owner.is_empty() || r.owner.eq_ignore_ascii_case(&address))
                .map(SandboxSummary::from)
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "sandboxes": summaries }))).into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Provision progress endpoints
// ---------------------------------------------------------------------------

async fn get_provision(
    _auth: SessionAuth,
    Path(call_id): Path<u64>,
) -> impl IntoResponse {
    match provision_progress::get_provision(call_id) {
        Ok(Some(status)) => match serde_json::to_value(status) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Ok(None) => api_error(StatusCode::NOT_FOUND, "Provision not found").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_provisions(_auth: SessionAuth) -> impl IntoResponse {
    match provision_progress::list_all_provisions() {
        Ok(provisions) => (StatusCode::OK, Json(serde_json::json!({ "provisions": provisions }))).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Session auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionRequest {
    nonce: String,
    signature: String,
}

async fn create_challenge() -> impl IntoResponse {
    let challenge = session_auth::create_challenge();
    match serde_json::to_value(challenge) {
        Ok(val) => (StatusCode::OK, Json(val)).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_session(Json(req): Json<SessionRequest>) -> impl IntoResponse {
    match session_auth::exchange_signature_for_token(&req.nonce, &req.signature) {
        Ok(token) => match serde_json::to_value(token) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => api_error(StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Secret provisioning endpoints (2-phase)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct InjectSecretsRequest {
    env_json: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct SecretsResponse {
    status: String,
    sandbox_id: String,
}

async fn inject_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(body): Json<InjectSecretsRequest>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    match secret_provisioning::inject_secrets(&sandbox_id, body.env_json, None).await {
        Ok(record) => (
            StatusCode::OK,
            Json(SecretsResponse {
                status: "secrets_configured".to_string(),
                sandbox_id: record.id,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn wipe_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = secret_provisioning::validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    match secret_provisioning::wipe_secrets(&sandbox_id, None).await {
        Ok(record) => (
            StatusCode::OK,
            Json(SecretsResponse {
                status: "secrets_wiped".to_string(),
                sandbox_id: record.id,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Health & metrics endpoints (unauthenticated)
// ---------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    let m = metrics::metrics();
    let active = m.active_sandboxes.load(std::sync::atomic::Ordering::Relaxed);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "uptime_secs": metrics::uptime_secs(),
            "active_sandboxes": active,
        })),
    )
}

async fn prometheus_metrics() -> impl IntoResponse {
    let body = metrics::metrics().render_prometheus();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

// ---------------------------------------------------------------------------
// Auth middleware helper (legacy — prefer `SessionAuth` extractor)
// ---------------------------------------------------------------------------

/// Validate the Authorization header and return the session claims.
///
/// **Prefer** using the [`SessionAuth`](crate::session_auth::SessionAuth) Axum
/// extractor directly in handler signatures instead of calling this manually.
pub fn extract_session_from_headers(headers: &HeaderMap) -> Result<session_auth::SessionClaims, (StatusCode, Json<ApiError>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = session_auth::extract_bearer_token(auth_header)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "Invalid Authorization header format"))?;

    session_auth::validate_session_token(token)
        .map_err(|e| api_error(StatusCode::UNAUTHORIZED, e.to_string()))
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

/// Build CORS layer from `CORS_ALLOWED_ORIGINS` env var.
///
/// - If the env var is set, parse comma-separated origins and whitelist them.
/// - If unset or `"*"`, allow any origin (development mode).
fn build_cors_layer() -> CorsLayer {
    use axum::http::{header, Method};

    let allowed_methods = vec![
        Method::GET,
        Method::POST,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let allowed_headers = vec![
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
        header::ACCEPT,
    ];

    let origins_env = std::env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default();

    if origins_env.is_empty() || origins_env == "*" {
        CorsLayer::new()
            .allow_origin(AllowOrigin::any())
            .allow_methods(allowed_methods)
            .allow_headers(allowed_headers)
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
// Router builder
// ---------------------------------------------------------------------------

/// Build the operator API router with all endpoints, CORS, and rate limiting.
///
/// For TEE-enabled operators, use [`operator_api_router_with_tee`] instead.
pub fn operator_api_router() -> Router {
    operator_api_router_with_tee(None)
}

/// Build the operator API router with optional TEE sealed secrets endpoints.
///
/// When `tee` is `Some(backend)`, the following endpoints are added:
/// - `GET  /api/sandboxes/{id}/tee/public-key`
/// - `POST /api/sandboxes/{id}/tee/sealed-secrets`
///
/// When `tee` is `None`, those routes are not registered and the router
/// behaves identically to [`operator_api_router`].
pub fn operator_api_router_with_tee(
    tee: Option<std::sync::Arc<dyn crate::tee::TeeBackend>>,
) -> Router {
    let cors = build_cors_layer();

    // Read endpoints: 120 req/min per IP
    let read_routes = Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .route("/api/provisions", get(list_provisions))
        .route("/api/provisions/{call_id}", get(get_provision))
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    // Write endpoints: 30 req/min per IP
    let write_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/secrets",
            post(inject_secrets).delete(wipe_secrets),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Auth endpoints: 10 req/min per IP (stricter to prevent brute-force)
    let auth_routes = Router::new()
        .route("/api/auth/challenge", post(create_challenge))
        .route("/api/auth/session", post(create_session))
        .layer(middleware::from_fn(rate_limit::auth_rate_limit));

    let mut router = Router::new()
        // Health & metrics (unauthenticated, no rate limiting)
        .route("/health", get(health))
        .route("/metrics", get(prometheus_metrics))
        .merge(read_routes)
        .merge(write_routes)
        .merge(auth_routes);

    // TEE sealed secrets endpoints (only when backend is configured)
    if let Some(backend) = tee {
        let tee_routes = Router::new()
            .route(
                "/api/sandboxes/{sandbox_id}/tee/public-key",
                get(crate::tee::sealed_secrets_api::get_tee_public_key),
            )
            .route(
                "/api/sandboxes/{sandbox_id}/tee/sealed-secrets",
                post(crate::tee::sealed_secrets_api::inject_sealed_secrets),
            )
            .layer(axum::Extension(Some(backend) as Option<std::sync::Arc<dyn crate::tee::TeeBackend>>))
            .layer(middleware::from_fn(rate_limit::write_rate_limit));

        router = router.merge(tee_routes);
    }

    router.layer(cors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    use std::sync::Once;
    static INIT: Once = Once::new();
    fn init() {
        INIT.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("operator-api-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            unsafe { std::env::set_var("BLUEPRINT_STATE_DIR", dir) };
        });
    }

    fn app() -> Router {
        operator_api_router()
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn test_auth_header() -> String {
        let token = session_auth::create_test_token("0x1234567890abcdef1234567890abcdef12345678");
        format!("Bearer {token}")
    }

    #[tokio::test]
    async fn test_list_sandboxes_empty() {
        init();

        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes")
                    .header("authorization", test_auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert!(json["sandboxes"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_list_sandboxes_requires_auth() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_provisions_empty() {
        init();

        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/provisions")
                    .header("authorization", test_auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert!(json["provisions"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_get_provision_not_found() {
        init();

        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/provisions/999999")
                    .header("authorization", test_auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_provision_lifecycle() {
        init();

        let auth = test_auth_header();
        // Start a provision
        let call_id = 77777;
        provision_progress::start_provision(call_id).unwrap();

        // Should be retrievable
        let response = app()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/provisions/{call_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["phase"], "queued");
        assert_eq!(json["progress_pct"], 0);

        // Update to ImagePull
        provision_progress::update_provision(
            call_id,
            provision_progress::ProvisionPhase::ImagePull,
            Some("Pulling image".into()),
            None,
        )
        .unwrap();

        let response = app()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/provisions/{call_id}"))
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let json = body_json(response.into_body()).await;
        assert_eq!(json["phase"], "image_pull");
        assert_eq!(json["progress_pct"], 20);

        // Clean up: move to terminal state so we don't pollute other tests
        provision_progress::update_provision(
            call_id,
            provision_progress::ProvisionPhase::Ready,
            Some("Done".into()),
            None,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_auth_challenge_returns_nonce() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/challenge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert!(json["nonce"].is_string());
        assert!(json["message"].is_string());
        assert!(json["expires_at"].is_number());
        assert!(json["nonce"].as_str().unwrap().len() == 64); // 32 bytes hex
    }

    #[tokio::test]
    async fn test_auth_session_invalid_sig() {
        // First get a challenge
        let response = app()
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/challenge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let challenge = body_json(response.into_body()).await;
        let nonce = challenge["nonce"].as_str().unwrap();

        // Submit with an invalid signature
        let body = serde_json::json!({
            "nonce": nonce,
            "signature": "0xdeadbeef"
        });

        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/session")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["status"], "ok");
        assert!(json["uptime_secs"].is_number());
        assert!(json["active_sandboxes"].is_number());
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("text/plain"));

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("sandbox_total_jobs"));
        assert!(body.contains("sandbox_active_sandboxes"));
    }

    #[tokio::test]
    async fn test_cors_preflight() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/sandboxes")
                    .header("origin", "http://localhost:5173")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("access-control-allow-origin"));
    }

    // ── TEE sealed secrets API tests ──────────────────────────────────────

    fn tee_app() -> Router {
        let mock = std::sync::Arc::new(
            crate::tee::mock::MockTeeBackend::new(crate::tee::TeeType::Tdx),
        );
        operator_api_router_with_tee(Some(mock))
    }

    /// Insert a sandbox record with TEE fields into the store.
    fn insert_tee_sandbox(id: &str, deployment_id: &str, owner: &str) {
        init();
        use crate::runtime::{SandboxRecord, SandboxState, sandboxes};
        sandboxes()
            .unwrap()
            .insert(
                id.to_string(),
                SandboxRecord {
                    id: id.to_string(),
                    container_id: format!("tee-{deployment_id}"),
                    sidecar_url: "http://mock-tee:8080".into(),
                    sidecar_port: 8080,
                    ssh_port: None,
                    token: "test-token".into(),
                    created_at: 1_700_000_000,
                    cpu_cores: 2,
                    memory_mb: 4096,
                    state: SandboxState::Running,
                    idle_timeout_seconds: 1800,
                    max_lifetime_seconds: 86400,
                    last_activity_at: 1_700_000_000,
                    stopped_at: None,
                    snapshot_image_id: None,
                    snapshot_s3_url: None,
                    container_removed_at: None,
                    image_removed_at: None,
                    original_image: "test:latest".into(),
                    base_env_json: "{}".into(),
                    user_env_json: String::new(),
                    snapshot_destination: None,
                    tee_deployment_id: Some(deployment_id.to_string()),
                    tee_metadata_json: Some(r#"{"backend":"mock"}"#.into()),
                    name: "tee-sandbox".into(),
                    agent_identifier: String::new(),
                    metadata_json: "{}".into(),
                    disk_gb: 50,
                    stack: String::new(),
                    owner: owner.to_string(),
                    tee_config: Some(crate::tee::TeeConfig {
                        required: true,
                        tee_type: crate::tee::TeeType::Tdx,
                    }),
                },
            )
            .unwrap();
    }

    /// Insert a non-TEE sandbox into the store.
    fn insert_plain_sandbox(id: &str, owner: &str) {
        init();
        use crate::runtime::{SandboxRecord, SandboxState, sandboxes};
        sandboxes()
            .unwrap()
            .insert(
                id.to_string(),
                SandboxRecord {
                    id: id.to_string(),
                    container_id: format!("ctr-{id}"),
                    sidecar_url: "http://localhost:9999".into(),
                    sidecar_port: 9999,
                    ssh_port: None,
                    token: "plain-token".into(),
                    created_at: 1_700_000_000,
                    cpu_cores: 1,
                    memory_mb: 1024,
                    state: SandboxState::Running,
                    idle_timeout_seconds: 1800,
                    max_lifetime_seconds: 86400,
                    last_activity_at: 1_700_000_000,
                    stopped_at: None,
                    snapshot_image_id: None,
                    snapshot_s3_url: None,
                    container_removed_at: None,
                    image_removed_at: None,
                    original_image: "test:latest".into(),
                    base_env_json: "{}".into(),
                    user_env_json: String::new(),
                    snapshot_destination: None,
                    tee_deployment_id: None,
                    tee_metadata_json: None,
                    name: "plain-sandbox".into(),
                    agent_identifier: String::new(),
                    metadata_json: "{}".into(),
                    disk_gb: 10,
                    stack: String::new(),
                    owner: owner.to_string(),
                    tee_config: None,
                },
            )
            .unwrap();
    }

    // Use a distinct owner for TEE tests so sandbox inserts don't pollute
    // the test_list_sandboxes_empty assertion (which uses a different address).
    const TEE_TEST_OWNER: &str = "0xTEE0000000000000000000000000000000000001";

    #[tokio::test]
    async fn test_tee_public_key_success() {
        insert_tee_sandbox("tee-pk-1", "deploy-pk-1", TEE_TEST_OWNER);
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

        let response = tee_app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes/tee-pk-1/tee/public-key")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["sandbox_id"], "tee-pk-1");
        assert_eq!(json["public_key"]["algorithm"], "x25519-hkdf-sha256");
        assert!(json["public_key"]["attestation"]["tee_type"].is_string());
    }

    #[tokio::test]
    async fn test_tee_public_key_not_tee_sandbox() {
        insert_plain_sandbox("plain-pk-1", TEE_TEST_OWNER);
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

        let response = tee_app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes/plain-pk-1/tee/public-key")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_tee_public_key_nonexistent_sandbox() {
        init();
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(
                "0x1234567890abcdef1234567890abcdef12345678"
            )
        );

        let response = tee_app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes/nonexistent/tee/public-key")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // validate_secret_access returns FORBIDDEN for nonexistent sandboxes
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_tee_public_key_no_auth() {
        let response = tee_app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes/any/tee/public-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_tee_sealed_secrets_success() {
        insert_tee_sandbox("tee-ss-1", "deploy-ss-1", TEE_TEST_OWNER);
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(TEE_TEST_OWNER)
        );

        let body = serde_json::json!({
            "sealed_secret": {
                "algorithm": "x25519-xsalsa20-poly1305",
                "ciphertext": [0xDE, 0xAD],
                "nonce": [0xBE, 0xEF]
            }
        });

        let response = tee_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sandboxes/tee-ss-1/tee/sealed-secrets")
                    .header("authorization", &auth)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["sandbox_id"], "tee-ss-1");
        assert_eq!(json["success"], true);
        assert_eq!(json["secrets_count"], 3);
    }

    #[tokio::test]
    async fn test_tee_routes_absent_without_backend() {
        init();
        let auth = format!(
            "Bearer {}",
            session_auth::create_test_token(
                "0x1234567890abcdef1234567890abcdef12345678"
            )
        );

        // Use app() which has no TEE backend
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/sandboxes/any/tee/public-key")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Route should not exist → 404
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
