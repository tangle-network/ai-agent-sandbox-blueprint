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
use tower_http::cors::{Any, CorsLayer};

use crate::provision_progress;
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

fn api_error(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
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
        Ok(Some(status)) => (StatusCode::OK, Json(serde_json::to_value(status).unwrap())).into_response(),
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
    (StatusCode::OK, Json(serde_json::to_value(challenge).unwrap()))
}

async fn create_session(Json(req): Json<SessionRequest>) -> impl IntoResponse {
    match session_auth::exchange_signature_for_token(&req.nonce, &req.signature) {
        Ok(token) => (StatusCode::OK, Json(serde_json::to_value(token).unwrap())).into_response(),
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

    match secret_provisioning::inject_secrets(&sandbox_id, body.env_json).await {
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

    match secret_provisioning::wipe_secrets(&sandbox_id).await {
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
// Router builder
// ---------------------------------------------------------------------------

/// Build the operator API router with all endpoints and CORS support.
pub fn operator_api_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Sandbox management
        .route("/api/sandboxes", get(list_sandboxes))
        // Secret provisioning (2-phase)
        .route(
            "/api/sandboxes/{sandbox_id}/secrets",
            post(inject_secrets).delete(wipe_secrets),
        )
        // Provision progress
        .route("/api/provisions", get(list_provisions))
        .route("/api/provisions/{call_id}", get(get_provision))
        // Session auth
        .route("/api/auth/challenge", post(create_challenge))
        .route("/api/auth/session", post(create_session))
        .layer(cors)
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
}
