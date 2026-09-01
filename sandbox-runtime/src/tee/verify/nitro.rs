//! AWS Nitro attestation-document verification.
//!
//! The cryptographic verifier lives in the canonical `blueprint-tee` crate. This
//! module adapts its verified report to sandbox-runtime's fixed-size facts and
//! preserves the raw Nitro nonce for caller-controlled replay binding.

use super::VerifiedQuote;
use blueprint_tee::attestation::AttestationPolicy;
use blueprint_tee::attestation::providers::aws_nitro::NitroVerifier;
use ciborium::value::Value;
use coset::{CborSerializable, CoseSign1, TaggedCborSerializable};

/// Verify an AWS Nitro COSE_Sign1 document against the pinned AWS Nitro root.
pub(crate) fn verify_nitro(evidence: &[u8]) -> Result<VerifiedQuote, String> {
    verify_nitro_with_verifier(evidence, &NitroVerifier::new())
}

/// Keep root injection private to this module so production always uses the
/// canonical pinned AWS root. Tests use the canonical verifier's test-only root
/// override to exercise the full positive path without fabricating trust.
pub(crate) fn verify_nitro_with_verifier(
    evidence: &[u8],
    verifier: &NitroVerifier,
) -> Result<VerifiedQuote, String> {
    let verified = verifier
        .verify_document(evidence, &AttestationPolicy::production())
        .map_err(|e| format!("AWS Nitro verification failed: {e}"))?;

    let measurement = hex::decode(&verified.report().measurement.digest)
        .map_err(|e| format!("AWS Nitro verifier returned a non-hex PCR0 measurement: {e}"))?;
    if verified.report().measurement.algorithm != "sha384" || measurement.len() != 48 {
        return Err(format!(
            "AWS Nitro verifier returned an invalid PCR0 measurement: algorithm={}, bytes={}",
            verified.report().measurement.algorithm,
            measurement.len()
        ));
    }

    Ok(VerifiedQuote {
        measurement,
        report_data: extract_nitro_report_data(evidence)?,
    })
}

/// Extract the signed Nitro `nonce` after `NitroVerifier` authenticates the
/// same COSE payload. Nitro allows a nonce shorter than 64 bytes, while the
/// runtime's report-data contract is fixed at 64 bytes, so right-pad with zeroes.
fn extract_nitro_report_data(evidence: &[u8]) -> Result<Option<[u8; 64]>, String> {
    let sign1 = CoseSign1::from_tagged_slice(evidence)
        .or_else(|_| CoseSign1::from_slice(evidence))
        .map_err(|e| format!("AWS Nitro evidence is not a valid COSE_Sign1 document: {e}"))?;
    let payload = sign1
        .payload
        .as_deref()
        .ok_or_else(|| "AWS Nitro COSE_Sign1 has no payload".to_string())?;
    let document: Value = ciborium::de::from_reader(payload)
        .map_err(|e| format!("AWS Nitro attestation document is not valid CBOR: {e}"))?;
    let map = document
        .as_map()
        .ok_or_else(|| "AWS Nitro attestation document is not a CBOR map".to_string())?;
    let Some(nonce_value) = map
        .iter()
        .rev()
        .find_map(|(key, value)| (key.as_text() == Some("nonce")).then_some(value))
    else {
        return Ok(None);
    };
    let nonce = nonce_value
        .as_bytes()
        .ok_or_else(|| "AWS Nitro attestation nonce is not a CBOR byte string".to_string())?;
    if nonce.len() > 64 {
        return Err(format!(
            "AWS Nitro attestation nonce is {} bytes; the report-data limit is 64 bytes",
            nonce.len()
        ));
    }
    let mut report_data = [0_u8; 64];
    report_data[..nonce.len()].copy_from_slice(nonce);
    Ok(Some(report_data))
}
