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
use crate::session_auth;

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Serialize)]
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

async fn list_sandboxes() -> impl IntoResponse {
    match sandboxes().and_then(|s| s.values()) {
        Ok(records) => {
            let summaries: Vec<SandboxSummary> = records.iter().map(SandboxSummary::from).collect();
            (StatusCode::OK, Json(serde_json::json!({ "sandboxes": summaries }))).into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Provision progress endpoints
// ---------------------------------------------------------------------------

async fn get_provision(Path(call_id): Path<u64>) -> impl IntoResponse {
    match provision_progress::get_provision(call_id) {
        Some(status) => (StatusCode::OK, Json(serde_json::to_value(status).unwrap())).into_response(),
        None => api_error(StatusCode::NOT_FOUND, "Provision not found").into_response(),
    }
}

async fn list_provisions() -> impl IntoResponse {
    let provisions = provision_progress::list_all_provisions();
    Json(serde_json::json!({ "provisions": provisions }))
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
// Auth middleware helper
// ---------------------------------------------------------------------------

/// Validate the Authorization header and return the session claims.
/// For routes that require authentication.
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
        // Provision progress
        .route("/api/provisions", get(list_provisions))
        .route("/api/provisions/{call_id}", get(get_provision))
        // Session auth
        .route("/api/auth/challenge", post(create_challenge))
        .route("/api/auth/session", post(create_session))
        .layer(cors)
}
