//! Operator API endpoints for TEE sealed secrets.
//!
//! These endpoints are additive and do not modify existing secret provisioning
//! routes. They are only meaningful for TEE-backed sandboxes.
//!
//! - `GET  /api/sandboxes/{id}/tee/public-key`      — fetch TEE-bound public key
//! - `POST /api/sandboxes/{id}/tee/sealed-secrets`   — inject encrypted secrets
//! - `GET  /api/sandboxes/{id}/tee/attestation`      — fetch fresh attestation
//! - `POST /api/sandboxes/{id}/tee/attestation`      — fetch nonce-bound attestation
//!
//! This module is intentionally isolated — it can be removed without affecting
//! the existing operator API or 2-phase plaintext secret provisioning.

use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::sealed_secrets::{SealedSecret, TeePublicKey};
use super::{
    AttestationReport, AttestationVerification, TeeBackend, expected_measurements_from_env,
    verify_attestation,
};
use crate::operator_api::api_error;
use crate::runtime::get_sandbox_by_id;
use crate::secret_provisioning::validate_secret_access;
use crate::session_auth::SessionAuth;

/// Name of the env var that controls whether trust-granting releases require a
/// server-pinned enclave measurement.
const REQUIRE_PINNED_ENV: &str = "SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT";

/// Whether the server must refuse to release sealed-secret material / TEE public
/// keys when it has no pinned measurement to enforce against.
///
/// Default is `true` (fail-closed): out of the box, a TEE deployment with no
/// `SANDBOX_TEE_EXPECTED_MEASUREMENTS` allowlist cannot release trust-granting
/// material. Operators who genuinely want pure client-side verification must set
/// `SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT=false` explicitly to opt out.
pub fn require_pinned_measurement_from_env() -> bool {
    match std::env::var(REQUIRE_PINNED_ENV) {
        Ok(raw) => !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Whether the two trust-granting routes (`tee/public-key`,
/// `tee/sealed-secrets`) may be served given the current measurement-pinning
/// configuration. Used at startup to fail closed: with no allowlist and the
/// requirement left on (the default), these routes are not mounted at all so a
/// misconfigured operator cannot silently hand back unverified material.
pub fn release_routes_enabled() -> bool {
    !require_pinned_measurement_from_env() || !super::expected_measurements_from_env().is_empty()
}

/// Outcome of [`enforce_release_gate`]: `true` when the server verified the
/// attestation against a pinned measurement, `false` when release proceeded
/// under the explicit client-side-only trust model (allowlist absent and the
/// pinning requirement turned off).
type GateOutcome = Result<bool, axum::response::Response>;

/// Response for `GET /api/sandboxes/{id}/tee/public-key`.
#[derive(Serialize)]
struct PublicKeyResponse {
    sandbox_id: String,
    public_key: TeePublicKey,
    /// `false` means the server did NOT verify the enclave measurement (no
    /// allowlist pinned and the operator opted out of requiring one); the client
    /// MUST verify the embedded attestation itself before trusting this key.
    server_enforced: bool,
}

/// Server-side attestation gate for trust-granting operations (public-key
/// release, sealed-secret injection).
///
/// Trust model: secret confidentiality ultimately depends on the *client*
/// verifying the attestation before encrypting to the TEE key. But when the
/// operator has declared what a good enclave looks like via
/// `SANDBOX_TEE_EXPECTED_MEASUREMENTS`, the server CAN and MUST enforce it too,
/// so a forged/unverified report is refused server-side rather than relying on
/// every client to remember to check.
///
/// Fail-closed default: when no allowlist is configured the server cannot pin a
/// measurement. By default (`SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT` unset/true)
/// this is a hard `FORBIDDEN` — the most security-sensitive operation does not
/// silently degrade to unenforced. An operator who genuinely wants pure
/// client-side verification must set that var to `false`, in which case release
/// proceeds but the gate returns `Ok(false)` so the caller can surface
/// `server_enforced: false` to the client.
///
/// Replay protection: when the gate enforces, it generates a fresh random nonce
/// and requires the attestation to carry it in the hardware-signed report data
/// (`report_data_matched`). A stale/replayed genuine quote therefore cannot pass
/// the gate. Backends that cannot bind report data fail closed.
///
/// Returns `Ok(server_enforced)` when release may proceed, or an HTTP error
/// response.
///
/// `expected` is the operator-independent allowlist of known-good measurements,
/// snapshotted by the caller (from `expected_measurements_from_env()`) so the
/// async gate never reads process env while a request is in flight.
async fn enforce_release_gate(
    backend: &dyn TeeBackend,
    deployment_id: &str,
    expected: &[Vec<u8>],
) -> GateOutcome {
    if expected.is_empty() {
        // No operator-pinned measurement → the server has nothing to enforce
        // against. Fail closed unless the operator has explicitly opted into the
        // client-side-only trust model.
        if require_pinned_measurement_from_env() {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "TEE release refused: no server-pinned enclave measurement \
                 (SANDBOX_TEE_EXPECTED_MEASUREMENTS is unset). Pin an allowlist, or set \
                 SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT=false to accept client-side-only \
                 verification.",
            )
            .into_response());
        }
        // Explicit opt-out: release proceeds but is NOT server-verified. Make the
        // unenforced gate visible to operators and to the caller.
        tracing::warn!(
            deployment_id,
            "TEE release gate not enforced server-side: SANDBOX_TEE_EXPECTED_MEASUREMENTS is \
             unset and SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT=false. Release relies entirely on \
             client-side attestation verification."
        );
        return Ok(false);
    }

    // Replay protection: bind the release to a fresh, server-generated nonce so
    // a stale/replayed (but otherwise genuine) quote cannot pass the gate. The
    // freshness binding is only meaningful if the backend can embed the nonce in
    // the hardware-signed report data, so fail closed when it cannot.
    if !backend.supports_attestation_report_data() {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            format!(
                "TEE backend {:?} cannot bind a freshness nonce into the attestation report \
                 data; refusing to release sealed-secret material without replay protection",
                backend.tee_type()
            ),
        )
        .into_response());
    }

    let mut nonce = [0u8; 64];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);

    let att = backend
        .attestation(deployment_id, Some(nonce))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())?;
    let verification = verify_attestation(&att, &backend.tee_type(), expected, Some(&nonce));
    if verification.is_trusted() {
        Ok(true)
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            format!(
                "TEE attestation not verified server-side (verdict: {:?}); refusing to release \
                 sealed-secret material",
                verification.verdict
            ),
        )
        .into_response())
    }
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

#[cfg(test)]
mod tests {
    // These serial tests hold TEST_ENV_GUARD (a std Mutex) across the
    // `enforce_release_gate(...).await` on purpose: the guard must span the await
    // so no other test mutates the process env (EXPECTED_ENV / REQUIRE_PINNED_ENV)
    // while the gate under test reads it. Dropping the guard before the await
    // would reintroduce the cross-test env race these tests exist to rule out.
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::tee::TeeType;
    use crate::tee::mock::MockTeeBackend;

    const EXPECTED_ENV: &str = "SANDBOX_TEE_EXPECTED_MEASUREMENTS";

    /// With a pinned allowlist, the gate enforces server-side: a mock backend
    /// cannot produce a hardware-verified quote (no `tee-verify` here, and the
    /// dummy report has no real quote), so the verdict is never `Verified` and
    /// release MUST be refused (HTTP 403). This proves the verifier is wired to
    /// an actual trust decision, not decorative.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_unverified_report_when_pinned() {
        // Snapshot the pinned allowlist under the env guard, then drop the guard
        // before the async gate runs: `enforce_release_gate` takes the snapshot
        // by value, so the std mutex is never held across an `.await`.
        let expected = {
            let _g = crate::TEST_ENV_GUARD
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            unsafe {
                std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
            }
            let snapshot = expected_measurements_from_env();
            unsafe {
                std::env::remove_var(EXPECTED_ENV);
            }
            snapshot
        };
        assert!(!expected.is_empty(), "allowlist snapshot must be pinned");
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("unverified report must be refused");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// When the allowlist is pinned but the backend cannot bind a freshness
    /// nonce into the report data, the gate must fail closed (HTTP 403) rather
    /// than release against a quote with no replay protection.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_when_backend_cannot_bind_report_data() {
        use std::sync::atomic::Ordering;

        let expected = {
            let _g = crate::TEST_ENV_GUARD
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            unsafe {
                std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
            }
            let snapshot = expected_measurements_from_env();
            unsafe {
                std::env::remove_var(EXPECTED_ENV);
            }
            snapshot
        };
        let backend = MockTeeBackend::new(TeeType::Tdx);
        backend.support_report_data.store(false, Ordering::Relaxed);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("must refuse without replay protection");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // The gate must not even fetch an attestation it cannot bind.
        assert_eq!(backend.attestation_count.load(Ordering::Relaxed), 0);
    }

    /// Out of the box (no allowlist, requirement left at its default), the gate
    /// FAILS CLOSED: trust-granting release is refused with HTTP 403 rather than
    /// silently proceeding unenforced.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_refuses_when_unpinned_by_default() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        let expected: Vec<Vec<u8>> = Vec::new();
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let resp = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect_err("default config must refuse unpinned release");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // No attestation is fetched: the gate refuses before touching the backend.
        assert_eq!(
            backend
                .attestation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// Only with the explicit opt-out does the gate defer to the client-side
    /// verification boundary — and it reports `server_enforced == false` so the
    /// caller can surface the unenforced state instead of pretending it verified.
    #[tokio::test]
    #[serial_test::serial]
    async fn release_gate_defers_to_client_when_explicitly_opted_out() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::set_var(REQUIRE_PINNED_ENV, "false");
        }
        let expected: Vec<Vec<u8>> = Vec::new();
        let backend = MockTeeBackend::new(TeeType::Tdx);
        let enforced = enforce_release_gate(&backend, "mock-deploy-1", &expected)
            .await
            .expect("explicit opt-out lets release proceed");
        unsafe {
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        assert!(
            !enforced,
            "an unpinned release must report server_enforced == false"
        );
    }

    /// The startup guard mirrors the runtime gate: routes stay mounted only when
    /// a pin exists or the operator opted out.
    #[test]
    #[serial_test::serial]
    fn release_routes_enabled_tracks_config() {
        let _g = crate::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
        assert!(
            !release_routes_enabled(),
            "default + no allowlist must not serve trust-granting routes"
        );
        unsafe {
            std::env::set_var(EXPECTED_ENV, "0xdeadbeef");
        }
        assert!(
            release_routes_enabled(),
            "a pinned allowlist enables routes"
        );
        unsafe {
            std::env::remove_var(EXPECTED_ENV);
            std::env::set_var(REQUIRE_PINNED_ENV, "false");
        }
        assert!(
            release_routes_enabled(),
            "explicit opt-out enables routes without a pin"
        );
        unsafe {
            std::env::remove_var(REQUIRE_PINNED_ENV);
        }
    }
}
