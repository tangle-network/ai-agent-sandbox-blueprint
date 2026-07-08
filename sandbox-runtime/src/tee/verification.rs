//! Attestation verdict/verification types + quote-signature checks + expected measurements.

use super::*;

/// Outcome of cryptographically verifying an [`AttestationReport`].
///
/// IMPORTANT: this encodes the *real* verification state. Under the `tee-verify`
/// feature the quote signature chain IS verified against a hardware root (see
/// [`verify_quote_signature`] / [`verify`]), so [`AttestationVerdict::Verified`]
/// is reachable for genuine TDX/SEV-SNP quotes. Without that feature the chain
/// is never verified and `Verified` is unreachable. Either way, callers and UIs
/// MUST treat anything other than `Verified` as untrusted and must not present
/// the workload as hardware-attested.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum AttestationVerdict {
    /// Quote signature chained to a hardware root AND the measurement signed
    /// inside the quote matched a pinned expected value AND (if a freshness
    /// nonce was supplied) the signed report data carried it.
    Verified,
    /// Structurally well-formed but NOT cryptographically verified (bad/absent
    /// signature chain, expired/insufficient TCB, or replayed report data).
    Unverified { reason: String },
    /// Signature verified, but the measurement signed inside the quote matched
    /// none of the pinned expected measurements.
    MeasurementMismatch,
}

/// Detailed result of [`verify_attestation`], suitable for serialising to the
/// UI / on-chain so the *honest* trust state travels with the attestation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttestationVerification {
    pub verdict: AttestationVerdict,
    /// Whether the quote signature was verified against a hardware root of trust
    /// (Intel PCS/PCCS for TDX, AMD KDS for SEV-SNP, NSM for Nitro).
    pub signature_verified: bool,
    /// Whether the measurement matched a pinned expected value.
    pub measurement_matched: bool,
    /// Whether the caller-supplied freshness nonce matched the report data the
    /// hardware signed. `true` when no nonce was requested (nothing to bind);
    /// `false` when a nonce was requested but the signed report data didn't
    /// carry it (replay / mismatch), which forces a non-`Verified` verdict.
    pub report_data_matched: bool,
    /// Whether the report passed structural/type checks.
    pub structural_ok: bool,
}

impl AttestationVerification {
    /// True only when the attestation is cryptographically trustworthy.
    pub fn is_trusted(&self) -> bool {
        matches!(self.verdict, AttestationVerdict::Verified)
    }
}

/// The measurement a verified quote actually carried (signed by hardware).
///
/// Returned by [`verify_quote_signature`] so the trust decision binds the
/// measurement the *hardware* signed, not the operator-supplied
/// `AttestationReport.measurement`.
#[cfg_attr(not(feature = "tee-verify"), allow(dead_code))]
pub(crate) struct SignedQuoteFacts {
    /// Measurement extracted from inside the cryptographically verified quote.
    pub(crate) measurement: Vec<u8>,
    /// 64-byte report data the hardware signed (caller nonce binding).
    pub(crate) report_data: [u8; 64],
}

/// Verify a TEE quote's signature against the appropriate hardware root of
/// trust.
///
/// Returns the measurement + report_data the hardware signed on success, so the
/// caller can bind them. Fails closed: any backend that cannot chain the quote
/// to a hardware root the operator does not control returns `Err(reason)`.
///
/// The genuine per-backend implementations live in [`verify`] (Intel TDX via
/// `dcap-qvl` to the Intel SGX Root CA; AMD SEV-SNP via the `sev` crate to the
/// AMD ARK). They are compiled only under the `tee-verify` feature. Without that
/// feature this is the honest fail-closed stub: nothing can be hardware-verified
/// and [`verify_attestation`] therefore never reports `Verified`.
///
/// `now_secs` is the trusted current time used for collateral/TCB/CRL freshness
/// checks; production callers pass the system clock.
#[cfg(feature = "tee-verify")]
pub(crate) fn verify_quote_signature(
    report: &AttestationReport,
    now_secs: u64,
) -> Result<SignedQuoteFacts, String> {
    let verified = verify::verify_quote_signature(report, now_secs)?;
    Ok(SignedQuoteFacts {
        measurement: verified.measurement,
        report_data: verified.report_data,
    })
}

/// Fail-closed stub used when `tee-verify` is disabled. Verifying a TDX/SEV-SNP
/// quote requires validating the platform certificate chain (Intel PCS/PCCS
/// DCAP collateral; AMD KDS VCEK) against a hardware root; that machinery lives
/// behind the `tee-verify` feature. Without it, no attestation can be trusted.
#[cfg(not(feature = "tee-verify"))]
pub(crate) fn verify_quote_signature(
    report: &AttestationReport,
    _now_secs: u64,
) -> Result<SignedQuoteFacts, String> {
    Err(format!(
        "quote-signature verification unavailable for {:?}: build with the `tee-verify` feature to chain the quote to a hardware root of trust",
        report.tee_type
    ))
}

/// Operator-independent allowlist of expected enclave measurements, read from
/// `SANDBOX_TEE_EXPECTED_MEASUREMENTS` (comma/whitespace-separated hex).
///
/// Measurement pinning only adds security when the expected value comes from a
/// source the operator does NOT control (a verifying client, or on-chain
/// config) — otherwise a malicious operator forges both the measurement and the
/// expected value. An empty allowlist means "no expected measurement
/// configured", which can never match.
pub fn expected_measurements_from_env() -> Vec<Vec<u8>> {
    std::env::var("SANDBOX_TEE_EXPECTED_MEASUREMENTS")
        .ok()
        .map(|raw| {
            raw.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| hex::decode(s.trim().trim_start_matches("0x")).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Maximum age (seconds) accepted for an attestation that was NOT bound to a
/// freshness nonce. The nonce binding ([`verify_attestation`] with
/// `expected_report_data`) is the durable replay defense; this bound is
/// defense-in-depth for paths that legitimately cannot challenge (e.g. a
/// deploy-time report surfaced for display). 10 minutes is generous enough for
/// clock skew and slow attestation fetches while still rejecting a quote
/// captured hours/days earlier and replayed.
pub(crate) const MAX_ATTESTATION_AGE_SECS: u64 = 600;
