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

mod attestation;
mod keys;

pub use attestation::*;
pub use keys::*;

// tee-level attestation-nonce helpers the moved endpoint code reaches via `super::`.
use super::{decode_attestation_nonce_hex, pad_attestation_nonce};

#[cfg(test)]
mod tests;
