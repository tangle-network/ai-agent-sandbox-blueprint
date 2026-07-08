//! Extracted from operator_api.rs — auth route group.

use super::*;

// ---------------------------------------------------------------------------
// Session auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SessionRequest {
    pub(crate) nonce: String,
    pub(crate) signature: String,
}

pub(crate) async fn create_challenge() -> impl IntoResponse {
    let challenge = match session_auth::create_challenge() {
        Ok(c) => c,
        Err(e) => {
            return api_error(StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response();
        }
    };
    match serde_json::to_value(challenge) {
        Ok(val) => (StatusCode::OK, Json(val)).into_response(),
        Err(e) => json_serialization_error(e),
    }
}

pub(crate) async fn create_session(Json(req): Json<SessionRequest>) -> impl IntoResponse {
    match session_auth::exchange_signature_for_token(&req.nonce, &req.signature) {
        Ok(token) => match serde_json::to_value(token) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => json_serialization_error(e),
        },
        Err(crate::error::SandboxError::Unavailable(msg)) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, msg).into_response()
        }
        Err(e) => api_error(StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

/// Revoke the current session token.
pub(crate) async fn revoke_session(headers: HeaderMap) -> impl IntoResponse {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(session_auth::extract_bearer_token);

    match token {
        Some(t) => {
            let revoked = session_auth::revoke_session(t);
            if revoked {
                (StatusCode::OK, Json(json!({"revoked": true}))).into_response()
            } else {
                (
                    StatusCode::OK,
                    Json(json!({"revoked": false, "message": "Token not found in session store"})),
                )
                    .into_response()
            }
        }
        None => api_error(StatusCode::BAD_REQUEST, "Missing Authorization header").into_response(),
    }
}
