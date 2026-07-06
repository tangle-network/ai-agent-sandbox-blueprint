//! Bearer-token extraction, required-config validation, and the reusable Axum
//! `SessionAuth` extractor.

use super::*;

/// Extract a Bearer token from an Authorization header value.
pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    let mut parts = auth_header.split_whitespace();
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next()?;
    if token.trim().is_empty() {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(token.trim())
}

// ---------------------------------------------------------------------------
// Configuration validation
// ---------------------------------------------------------------------------

/// Validate that required configuration for session auth is present.
///
/// Checks that `SESSION_AUTH_SECRET` is set and non-empty. Without this,
/// PASETO tokens use a random key that changes on restart, silently breaking
/// all existing sessions.
///
/// Call this early in each binary's `main()` — in production it should be
/// treated as a hard error; in test mode, log a warning and continue.
pub fn validate_required_config() -> std::result::Result<(), String> {
    match std::env::var("SESSION_AUTH_SECRET") {
        Ok(mut val) => {
            let ok = !val.trim().is_empty();
            val.zeroize();
            if ok {
                Ok(())
            } else {
                Err("SESSION_AUTH_SECRET is set but empty. \
                     Provide a non-empty secret for stable session auth."
                    .to_string())
            }
        }
        Err(_) => Err("SESSION_AUTH_SECRET is not set. \
             Sessions will use a random key and break on restart. \
             Set this env var before starting the operator."
            .to_string()),
    }
}

// ---------------------------------------------------------------------------
// Axum extractor — reusable across any blueprint's operator API
// ---------------------------------------------------------------------------

/// Axum extractor that validates the `Authorization: Bearer <token>` header
/// and yields the authenticated wallet address.
///
/// Usage in handler:
/// ```ignore
/// async fn my_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse { ... }
/// ```
pub struct SessionAuth(pub String);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for SessionAuth {
    type Rejection = (axum::http::StatusCode, String);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    "Missing Authorization header".to_string(),
                )
            })?;

        let token = extract_bearer_token(auth_header).ok_or_else(|| {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format".to_string(),
            )
        })?;

        let claims = validate_session_token(token)
            .map_err(|e| (axum::http::StatusCode::UNAUTHORIZED, e.to_string()))?;

        Ok(SessionAuth(claims.address))
    }
}
