//! End-to-end attestation verification + sidecar health / key / sealed-secret operations.

use super::*;

/// Cryptographically verify an attestation report, returning the *honest*
/// verification state.
///
/// The verdict is [`AttestationVerdict::Verified`] only when BOTH the quote
/// signature is verified against a hardware root AND the measurement signed
/// inside the quote matches a pinned expected value. Under the `tee-verify`
/// feature this can genuinely reach `Verified`; without it, [`verify_quote_signature`]
/// always errs and the verdict can never be `Verified` — by design, so the
/// product never claims a guarantee it cannot back. This is the single entry
/// point any trust decision (UI badge, on-chain gate, sealed-secret release)
/// must consult; structural checks alone ([`validate_attestation_report`]) are
/// NOT a trust decision.
pub fn verify_attestation(
    report: &AttestationReport,
    expected_type: &TeeType,
    expected_measurements: &[Vec<u8>],
    expected_report_data: Option<&[u8; 64]>,
) -> AttestationVerification {
    verify_attestation_at(
        report,
        expected_type,
        expected_measurements,
        expected_report_data,
        crate::util::now_ts(),
    )
}

/// [`verify_attestation`] with an explicit trusted `now_secs`, so collateral/TCB
/// freshness is evaluated against a caller-controlled clock. Used by tests to
/// pin the time to a vendored quote's validity window; production uses the
/// system clock via [`verify_attestation`].
pub(crate) fn verify_attestation_at(
    report: &AttestationReport,
    expected_type: &TeeType,
    expected_measurements: &[Vec<u8>],
    expected_report_data: Option<&[u8; 64]>,
    now_secs: u64,
) -> AttestationVerification {
    let structural_ok = validate_attestation_report(report, expected_type).is_ok();
    let signature_result = verify_quote_signature(report, now_secs);
    let signature_verified = signature_result.is_ok();

    // Bind the measurement to the value the HARDWARE signed inside the quote,
    // not the operator-supplied `report.measurement`. A malicious operator can
    // forge `report.measurement` freely, but cannot forge the signed quote.
    // When the signature didn't verify we have no trustworthy measurement, so
    // the match is meaningless and we report `false`.
    let signed_measurement = signature_result.as_ref().ok().map(|f| &f.measurement);
    let measurement_matched = !expected_measurements.is_empty()
        && signed_measurement.is_some_and(|signed| {
            expected_measurements
                .iter()
                .any(|expected| expected == signed)
        });

    // Replay binding: when the caller challenged with a nonce, the nonce MUST
    // equal the report data the hardware signed. Compared in constant time. With
    // no challenge there is nothing to bind, so this is vacuously satisfied —
    // but only counts toward trust once the signature itself verified.
    let report_data_matched = match (expected_report_data, signature_result.as_ref().ok()) {
        (Some(expected), Some(facts)) => bool::from(subtle::ConstantTimeEq::ct_eq(
            &facts.report_data[..],
            &expected[..],
        )),
        (Some(_), None) => false,
        (None, _) => true,
    };

    // Staleness bound (defense-in-depth, only on the un-challenged path). With a
    // nonce, `report_data_matched` already proves freshness. Without one, reject
    // a report whose timestamp is too far in the past so a genuine-but-old quote
    // cannot be replayed. `report.timestamp` is operator-supplied and only
    // tightens (never relaxes) the decision: a future/forged timestamp does not
    // grant trust because the signature/measurement gates still apply.
    let fresh_enough = expected_report_data.is_some()
        || now_secs.saturating_sub(report.timestamp) <= MAX_ATTESTATION_AGE_SECS;

    let verdict = if !structural_ok {
        AttestationVerdict::Unverified {
            reason: "attestation report failed structural validation".to_string(),
        }
    } else if let Err(reason) = signature_result {
        AttestationVerdict::Unverified { reason }
    } else if !report_data_matched {
        AttestationVerdict::Unverified {
            reason: "freshness nonce did not match the report data signed by the hardware (possible replay)".to_string(),
        }
    } else if !fresh_enough {
        AttestationVerdict::Unverified {
            reason: format!(
                "attestation is stale: timestamp {} is more than {}s before now {} and no \
                 freshness nonce was supplied (possible replay)",
                report.timestamp, MAX_ATTESTATION_AGE_SECS, now_secs
            ),
        }
    } else if !measurement_matched {
        AttestationVerdict::MeasurementMismatch
    } else {
        AttestationVerdict::Verified
    };

    AttestationVerification {
        verdict,
        signature_verified,
        measurement_matched,
        report_data_matched,
        structural_ok,
    }
}

/// Poll a sidecar's `/health` endpoint until it responds successfully.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn wait_for_sidecar_health(
    sidecar_url: &str,
    token: &str,
    timeout: std::time::Duration,
) -> crate::error::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(crate::error::SandboxError::CloudProvider(
                "Sidecar health check timed out".into(),
            ));
        }
        if let (Ok(url), Ok(headers)) = (
            crate::http::build_url(sidecar_url, "/health"),
            crate::http::auth_headers(token),
        ) && crate::http::send_json(reqwest::Method::GET, url, None, headers)
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Derive a TEE-bound public key by proxying to the sidecar.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn sidecar_derive_public_key(
    deployment_id: &str,
) -> crate::error::Result<sealed_secrets::TeePublicKey> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let url = crate::http::build_url(&sidecar_url, "/tee/public-key")?;
    let headers = crate::http::auth_headers(&token)?;
    let (_status, body) = crate::http::send_json(reqwest::Method::GET, url, None, headers).await?;
    serde_json::from_str(&body).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid TeePublicKey response: {e}"))
    })
}

/// Inject sealed secrets by proxying to the sidecar.
#[allow(dead_code)] // Used by TEE backends
pub(crate) async fn sidecar_inject_sealed_secrets(
    deployment_id: &str,
    sealed: &sealed_secrets::SealedSecret,
) -> crate::error::Result<sealed_secrets::SealedSecretResult> {
    let (sidecar_url, token) = sidecar_info_for_deployment(deployment_id)?;
    let payload = serde_json::to_value(sealed).map_err(|e| {
        crate::error::SandboxError::Validation(format!("Failed to serialize sealed secret: {e}"))
    })?;
    let resp = crate::http::sidecar_post_json(&sidecar_url, "/tee/sealed-secrets", &token, payload)
        .await?;
    serde_json::from_value(resp).map_err(|e| {
        crate::error::SandboxError::Http(format!("Invalid SealedSecretResult response: {e}"))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Mock backend for tests
// ─────────────────────────────────────────────────────────────────────────────
