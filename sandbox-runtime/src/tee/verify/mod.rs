//! Quote verification against hardware roots. Compiled only with `tee-verify`.
//!
//! Never trust `report.measurement` from the caller. Read it from the verified
//! quote and require equality. SEV without a bundled KDS CRL does not check
//! revocation. Reject debug policy, VMPL != 0, and SEV version < 2.

use super::{AttestationReport, TeeType};

/// The trustworthy facts extracted from a cryptographically verified quote.
///
/// Returned by the per-backend verifiers so the caller can bind the measurement
/// and nonce that the *hardware* actually signed, rather than the operator-
/// supplied fields on [`AttestationReport`].
#[derive(Clone, Debug)]
pub(crate) struct VerifiedQuote {
    /// Enclave/TD measurement as signed inside the quote (MRTD for TDX,
    /// LAUNCH_DIGEST for SEV-SNP).
    pub measurement: Vec<u8>,
    /// The 64-byte report data the hardware signed, when the document carries
    /// a nonce. AWS Nitro documents may omit this field.
    pub report_data: Option<[u8; 64]>,
}

/// Verify a TEE quote's signature chain against the appropriate hardware root.
///
/// On success, returns the measurement + report_data the hardware signed. The
/// caller ([`super::verify_attestation`]) must still compare that measurement to
/// a pinned expected value before declaring trust.
///
/// Fails closed: any parse error, unsupported evidence shape, missing
/// collateral, broken chain, bad signature, expired/insufficient TCB, or
/// non-verifiable backend yields `Err(reason)` — never a silent `Ok`.
pub(crate) fn verify_quote_signature(
    report: &AttestationReport,
    now_secs: u64,
) -> Result<VerifiedQuote, String> {
    match report.tee_type {
        TeeType::Tdx => verify_tdx(&report.evidence, now_secs),
        TeeType::Sev => verify_sev(&report.evidence, now_secs),
        TeeType::Nitro => verify_nitro(&report.evidence),
        TeeType::None => Err("TeeType::None carries no hardware quote to verify".to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Evidence envelope
// ─────────────────────────────────────────────────────────────────────────────

/// Self-describing evidence wrapper carried in `AttestationReport.evidence`.
///
/// DCAP/SNP signature verification needs collateral (Intel PCS material for TDX;
/// the VCEK/VLEK leaf for SEV-SNP) that is NOT present in a bare on-device
/// report. Producers that can verify therefore wrap the quote and its
/// collateral in this JSON envelope. A bare (non-JSON) `evidence` blob is still
/// accepted as a raw quote where the format is self-contained enough to detect,
/// but is rejected when collateral is structurally required (see the per-backend
/// notes), because verifying a quote with no collateral would be dishonest.
#[derive(serde::Deserialize)]
struct EvidenceEnvelope {
    /// Hex-encoded primary blob: a DCAP quote (TDX) or an SNP report (SEV).
    #[serde(default)]
    quote: Option<String>,
    /// Hex-encoded SNP report (SEV alias for `quote`).
    #[serde(default)]
    report: Option<String>,
    /// Intel PCS collateral (TDX), as accepted by `dcap-qvl`.
    #[serde(default)]
    collateral: Option<dcap_qvl::QuoteCollateralV3>,
    /// Hex-encoded DER VCEK leaf certificate (SEV).
    #[serde(default)]
    vcek: Option<String>,
    /// Hex-encoded DER VLEK leaf certificate (SEV, alternative to VCEK).
    #[serde(default)]
    vlek: Option<String>,
    /// AMD CPU generation for SEV root selection: "milan" | "genoa" | "turin".
    #[serde(default)]
    amd_generation: Option<String>,
    /// Hex-encoded DER AMD KDS CRL for the VEK issuer (SEV). When present, the
    /// VCEK/VLEK serial is checked against it after the CRL signature is verified
    /// against the pinned ASK. When absent, revocation is not checked and SEV is
    /// lower-assurance than TDX (whose DCAP collateral always carries CRLs).
    #[serde(default)]
    crl: Option<String>,
}

fn parse_envelope(evidence: &[u8]) -> Option<EvidenceEnvelope> {
    // Only attempt JSON when the blob actually looks like a JSON object, so we
    // don't misclassify binary quotes as malformed JSON.
    let first = evidence.iter().find(|b| !b.is_ascii_whitespace())?;
    if *first != b'{' {
        return None;
    }
    serde_json::from_slice::<EvidenceEnvelope>(evidence).ok()
}

fn decode_hex(label: &str, value: &str) -> Result<Vec<u8>, String> {
    hex::decode(value.trim().trim_start_matches("0x"))
        .map_err(|e| format!("{label} is not valid hex: {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Intel TDX (DCAP)
// ─────────────────────────────────────────────────────────────────────────────

/// TDX DCAP quote header constants used only to give precise rejection reasons.
const TDX_QUOTE_TEE_TYPE: u32 = 0x0000_0081;
const SGX_QUOTE_TEE_TYPE: u32 = 0x0000_0000;
/// Size of a local on-device TDREPORT (what `/dev/tdx_guest` emits). Kept here
/// (rather than reaching into the `tee-direct`-gated `attestation` module) so
/// this module compiles independently of which backends are enabled.
const TDX_TDREPORT_SIZE: usize = 1024;

mod certs;
mod nitro;
mod sev_snp;

#[cfg(test)]
pub(crate) use nitro::verify_nitro_with_verifier;
mod tdx;

pub(crate) use certs::*;
pub(crate) use nitro::*;
pub(crate) use sev_snp::*;
pub(crate) use tdx::*;

/// Derive a `now` that sits inside a DCAP collateral's validity window, used by
/// the TDX positive tests here and the end-to-end tests in `super`. Returns
/// `max(TCB.issueDate, QE.issueDate) + 60s`, which is just past the latest
/// collateral opening and therefore before every `nextUpdate` (TCB, QE, and the
/// PCK CRL all share the ~30-day window that opens at the issue dates).
#[cfg(test)]
pub(crate) fn tests_now_from_collateral(collateral_json: &[u8]) -> u64 {
    let collateral: dcap_qvl::QuoteCollateralV3 =
        serde_json::from_slice(collateral_json).expect("collateral json");
    let issue_secs = |json: &str| -> u64 {
        let v: serde_json::Value = serde_json::from_str(json).expect("collateral field json");
        let issue = v["issueDate"].as_str().expect("issueDate");
        chrono::DateTime::parse_from_rfc3339(issue)
            .expect("parse issueDate")
            .timestamp() as u64
    };
    issue_secs(&collateral.tcb_info).max(issue_secs(&collateral.qe_identity)) + 60
}

#[cfg(test)]
mod tests;
