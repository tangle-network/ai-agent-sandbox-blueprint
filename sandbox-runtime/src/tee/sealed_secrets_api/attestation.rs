//! TEE attestation challenge/response endpoints.

use super::*;

/// Response for `GET /api/sandboxes/{id}/tee/attestation`.
///
/// `verification` is the *server-evaluated* trust state for the report (quote
/// signature chained to a hardware root, measurement pinned via
/// `SANDBOX_TEE_EXPECTED_MEASUREMENTS`, and — when a nonce was supplied — the
/// freshness binding). It travels with the raw report so a consumer cannot
/// accidentally trust an unverified report. Clients SHOULD still verify
/// independently before encrypting secrets, but the honest verdict is no longer
/// optional or hidden.
#[derive(Serialize)]
struct AttestationResponse {
    sandbox_id: String,
    attestation: AttestationReport,
    verification: AttestationVerification,
}

/// Request body for `POST /api/sandboxes/{id}/tee/attestation`.
#[derive(Deserialize)]
pub struct AttestationChallengeRequest {
    /// Hex-encoded 32-64 byte caller nonce. Accepted with or without `0x`.
    attestation_nonce: String,
}

/// `GET /api/sandboxes/{sandbox_id}/tee/attestation`
///
/// Returns a fresh attestation report from the TEE backend for the sandbox.
/// Allows users to request attestation at any time, not just during deploy.
pub async fn get_tee_attestation(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    tee_backend: axum::Extension<Option<Arc<dyn TeeBackend>>>,
) -> impl IntoResponse {
    tee_attestation_response(address, sandbox_id, tee_backend, None).await
}

/// `POST /api/sandboxes/{sandbox_id}/tee/attestation`
///
/// Returns a fresh attestation report bound to caller-supplied report data.
/// This protects against replay when the selected backend supports native
/// TDX/SEV-SNP report data.
pub async fn post_tee_attestation(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    tee_backend: axum::Extension<Option<Arc<dyn TeeBackend>>>,
    Json(body): Json<AttestationChallengeRequest>,
) -> impl IntoResponse {
    let report_data = match super::decode_attestation_nonce_hex(&body.attestation_nonce)
        .and_then(|nonce| super::pad_attestation_nonce(&nonce))
    {
        Ok(data) => data,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    tee_attestation_response(address, sandbox_id, tee_backend, report_data).await
}

async fn tee_attestation_response(
    address: String,
    sandbox_id: String,
    tee_backend: axum::Extension<Option<Arc<dyn TeeBackend>>>,
    report_data: Option<[u8; 64]>,
) -> axum::response::Response {
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

    if report_data.is_some() && !backend.supports_attestation_report_data() {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "TEE backend {:?} does not support caller-supplied attestation nonces",
                backend.tee_type()
            ),
        )
        .into_response();
    }

    match backend.attestation(&deployment_id, report_data).await {
        Ok(att) => {
            // Evaluate the honest trust state server-side. The expected type is
            // the backend's own TEE type; expected measurements come from the
            // operator-independent allowlist; and when the caller supplied a
            // nonce we bind it here (the report data the hardware signed must
            // carry it) rather than merely echoing it back.
            let verification = verify_attestation(
                &att,
                &backend.tee_type(),
                &expected_measurements_from_env(),
                report_data.as_ref(),
            );
            (
                StatusCode::OK,
                Json(AttestationResponse {
                    sandbox_id,
                    attestation: att,
                    verification,
                }),
            )
                .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
