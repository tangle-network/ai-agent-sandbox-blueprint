//! Public-key + sealed-secret injection endpoints.

use super::*;

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
    /// `false` means the encrypted blob was forwarded WITHOUT server-side
    /// attestation verification (no allowlist pinned, operator opted out). The
    /// client must have verified the enclave before sealing to its key.
    server_enforced: bool,
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
                .into_response();
        }
    };

    let backend = match tee_backend.as_ref() {
        Some(b) => b.as_ref(),
        None => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "TEE backend not configured",
            )
            .into_response();
        }
    };

    let server_enforced = match enforce_release_gate(
        backend,
        &deployment_id,
        &expected_measurements_from_env(),
    )
    .await
    {
        Ok(enforced) => enforced,
        Err(resp) => return resp,
    };

    match backend.derive_public_key(&deployment_id).await {
        Ok(pk) => (
            StatusCode::OK,
            Json(PublicKeyResponse {
                sandbox_id,
                public_key: pk,
                server_enforced,
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
                .into_response();
        }
    };

    let backend = match tee_backend.as_ref() {
        Some(b) => b.as_ref(),
        None => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "TEE backend not configured",
            )
            .into_response();
        }
    };

    let server_enforced = match enforce_release_gate(
        backend,
        &deployment_id,
        &expected_measurements_from_env(),
    )
    .await
    {
        Ok(enforced) => enforced,
        Err(resp) => return resp,
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
                server_enforced,
                error: result.error,
            }),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
