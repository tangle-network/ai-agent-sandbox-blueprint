//! Operator API endpoints for TEE sealed secrets.
//!
//! These endpoints are additive and do not modify existing secret provisioning
//! routes. They are only meaningful for TEE-backed sandboxes.
//!
//! - `GET  /api/sandboxes/{id}/tee/public-key`      — fetch TEE-bound public key
//! - `POST /api/sandboxes/{id}/tee/sealed-secrets`   — inject encrypted secrets
//!
//! This module is intentionally isolated — it can be removed without affecting
//! the existing operator API or 2-phase plaintext secret provisioning.

use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::sealed_secrets::{SealedSecret, TeePublicKey};
use super::TeeBackend;
use crate::operator_api::api_error;
use crate::runtime::get_sandbox_by_id;
use crate::secret_provisioning::validate_secret_access;
use crate::session_auth::SessionAuth;

/// Response for `GET /api/sandboxes/{id}/tee/public-key`.
#[derive(Serialize)]
struct PublicKeyResponse {
    sandbox_id: String,
    public_key: TeePublicKey,
}

/// Request body for `POST /api/sandboxes/{id}/tee/sealed-secrets`.
#[derive(Deserialize)]
pub struct InjectSealedRequest {
    sealed_secret: SealedSecret,
}

/// Response for `POST /api/sandboxes/{id}/tee/sealed-secrets`.
#[derive(Serialize)]
struct SealedSecretResponse {
    sandbox_id: String,
    success: bool,
    secrets_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// `GET /api/sandboxes/{sandbox_id}/tee/public-key`
///
/// Returns the TEE-bound public key for the sandbox's enclave.
/// The client verifies the embedded attestation, then encrypts
/// secrets to this key.
pub async fn get_tee_public_key(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    tee_backend: axum::Extension<Option<Arc<dyn TeeBackend>>>,
) -> impl IntoResponse {
    if let Err(e) = validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    let record = match get_sandbox_by_id(&sandbox_id) {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };

    let deployment_id = match &record.tee_deployment_id {
        Some(id) => id.clone(),
        None => {
            return api_error(StatusCode::BAD_REQUEST, "Sandbox is not a TEE deployment")
                .into_response()
        }
    };

    let backend = match tee_backend.as_ref() {
        Some(b) => b.as_ref(),
        None => {
            return api_error(StatusCode::SERVICE_UNAVAILABLE, "TEE backend not configured")
                .into_response()
        }
    };

    match backend.derive_public_key(&deployment_id).await {
        Ok(pk) => (
            StatusCode::OK,
            Json(PublicKeyResponse {
                sandbox_id,
                public_key: pk,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `POST /api/sandboxes/{sandbox_id}/tee/sealed-secrets`
///
/// Accepts an encrypted secret blob and forwards it to the TEE sidecar
/// for decryption and injection. The operator never sees plaintext.
pub async fn inject_sealed_secrets(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    tee_backend: axum::Extension<Option<Arc<dyn TeeBackend>>>,
    Json(body): Json<InjectSealedRequest>,
) -> impl IntoResponse {
    if let Err(e) = validate_secret_access(&sandbox_id, &address) {
        return api_error(StatusCode::FORBIDDEN, e.to_string()).into_response();
    }

    let record = match get_sandbox_by_id(&sandbox_id) {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };

    let deployment_id = match &record.tee_deployment_id {
        Some(id) => id.clone(),
        None => {
            return api_error(StatusCode::BAD_REQUEST, "Sandbox is not a TEE deployment")
                .into_response()
        }
    };

    let backend = match tee_backend.as_ref() {
        Some(b) => b.as_ref(),
        None => {
            return api_error(StatusCode::SERVICE_UNAVAILABLE, "TEE backend not configured")
                .into_response()
        }
    };

    match backend
        .inject_sealed_secrets(&deployment_id, &body.sealed_secret)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(SealedSecretResponse {
                sandbox_id,
                success: result.success,
                secrets_count: result.secrets_count,
                error: result.error,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
