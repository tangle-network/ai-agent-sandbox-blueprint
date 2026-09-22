//! AWS Nitro attestation-document verification.
//!
//! The cryptographic verifier lives in the canonical `blueprint-tee` crate. This
//! module adapts its verified report to sandbox-runtime's fixed-size facts and
//! preserves the raw Nitro nonce for caller-controlled replay binding.

use super::VerifiedQuote;
use blueprint_tee::attestation::AttestationPolicy;
use blueprint_tee::attestation::providers::aws_nitro::NitroVerifier;
use ciborium::value::Value;
use coset::{CborSerializable, CoseSign1, TaggedCborSerializable, iana};
use x509_cert::{
    Certificate,
    der::{Decode, Encode},
    ext::pkix::{BasicConstraints, KeyUsage},
};

const MAX_NITRO_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_NITRO_CERT_BYTES: usize = 1024;
const MAX_NITRO_NONCE_BYTES: usize = 64;
const COSE_ES384_SIGNATURE_BYTES: usize = 96;

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
    let report_data = parse_nitro_evidence(evidence)?;
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
        report_data,
    })
}

/// Parse and validate the AWS document syntax before the canonical verifier
/// performs its root-chain and signature checks. The upstream verifier owns
/// those cryptographic operations; this boundary enforces AWS field types and
/// the runtime's fixed-size report-data contract.
fn parse_nitro_evidence(evidence: &[u8]) -> Result<Option<[u8; 64]>, String> {
    let sign1 = CoseSign1::from_tagged_slice(evidence)
        .or_else(|_| CoseSign1::from_slice(evidence))
        .map_err(|e| format!("AWS Nitro evidence is not a valid COSE_Sign1 document: {e}"))?;
    if !matches!(
        sign1.protected.header.alg.as_ref(),
        Some(coset::RegisteredLabelWithPrivate::Assigned(
            iana::Algorithm::ES384
        ))
    ) {
        return Err("AWS Nitro COSE_Sign1 must use protected ES384 (alg -35)".to_string());
    }
    if sign1.signature.len() != COSE_ES384_SIGNATURE_BYTES {
        return Err(format!(
            "AWS Nitro COSE_Sign1 ES384 signature must be {COSE_ES384_SIGNATURE_BYTES} bytes, got {}",
            sign1.signature.len()
        ));
    }
    let payload = sign1
        .payload
        .as_deref()
        .ok_or_else(|| "AWS Nitro COSE_Sign1 has no payload".to_string())?;
    if !(1..=MAX_NITRO_PAYLOAD_BYTES).contains(&payload.len()) {
        return Err(format!(
            "AWS Nitro COSE_Sign1 payload must be 1-{MAX_NITRO_PAYLOAD_BYTES} bytes, got {}",
            payload.len()
        ));
    }
    let mut payload_reader = payload;
    let document: Value = ciborium::de::from_reader(&mut payload_reader)
        .map_err(|e| format!("AWS Nitro attestation document is not valid CBOR: {e}"))?;
    if !payload_reader.is_empty() {
        return Err("AWS Nitro attestation payload contains trailing CBOR data".to_string());
    }
    let map = document
        .as_map()
        .ok_or_else(|| "AWS Nitro attestation document is not a CBOR map".to_string())?;

    let mut seen = std::collections::BTreeSet::new();
    let mut module_id = None;
    let mut digest = None;
    let mut timestamp = None;
    let mut pcrs = None;
    let mut certificate = None;
    let mut cabundle = None;
    let mut nonce = None;

    for (key, value) in map {
        let key = key
            .as_text()
            .ok_or_else(|| "AWS Nitro attestation field name is not text".to_string())?;
        if !seen.insert(key) {
            return Err(format!(
                "AWS Nitro attestation contains duplicate field {key:?}"
            ));
        }
        match key {
            "module_id" => module_id = Some(require_non_empty_text(value, "module_id")?),
            "digest" => {
                let value = require_text(value, "digest")?;
                if value != "SHA384" {
                    return Err(format!(
                        "AWS Nitro attestation digest must be SHA384, got {value:?}"
                    ));
                }
                digest = Some(value);
            }
            "timestamp" => {
                let value = value
                    .as_integer()
                    .and_then(|value| u64::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        "AWS Nitro attestation timestamp must be a positive integer".to_string()
                    })?;
                timestamp = Some(value);
            }
            "pcrs" => pcrs = Some(validate_nitro_pcrs(value)?),
            "certificate" => {
                certificate = Some(require_bounded_bytes(
                    value,
                    "certificate",
                    1..=MAX_NITRO_CERT_BYTES,
                )?);
            }
            "cabundle" => {
                let entries = value
                    .as_array()
                    .ok_or_else(|| "AWS Nitro cabundle must be an array".to_string())?;
                if entries.is_empty() {
                    return Err("AWS Nitro cabundle must contain at least one certificate".into());
                }
                let mut bundle = Vec::with_capacity(entries.len());
                for entry in entries {
                    bundle.push(require_bounded_bytes(
                        entry,
                        "cabundle certificate",
                        1..=MAX_NITRO_CERT_BYTES,
                    )?);
                }
                cabundle = Some(bundle);
            }
            "public_key" => {
                require_bounded_bytes(value, "public_key", 1..=MAX_NITRO_CERT_BYTES)?;
            }
            "user_data" => {
                require_bounded_bytes(value, "user_data", 0..=512)?;
            }
            "nonce" => {
                nonce = Some(require_bounded_bytes(
                    value,
                    "nonce",
                    0..=MAX_NITRO_NONCE_BYTES,
                )?);
            }
            other => {
                return Err(format!(
                    "AWS Nitro attestation contains unsupported field {other:?}"
                ));
            }
        }
    }

    let _module_id =
        module_id.ok_or_else(|| "AWS Nitro attestation missing module_id".to_string())?;
    let _digest = digest.ok_or_else(|| "AWS Nitro attestation missing digest".to_string())?;
    let _timestamp =
        timestamp.ok_or_else(|| "AWS Nitro attestation missing timestamp".to_string())?;
    let pcrs = pcrs.ok_or_else(|| "AWS Nitro attestation missing pcrs".to_string())?;
    let certificate =
        certificate.ok_or_else(|| "AWS Nitro attestation missing certificate".to_string())?;
    let cabundle = cabundle.ok_or_else(|| "AWS Nitro attestation missing cabundle".to_string())?;
    if pcrs.get(&0).is_none() {
        return Err("AWS Nitro attestation must contain PCR0".to_string());
    }
    validate_nitro_certificate_usage(&certificate, &cabundle)?;

    let report_data = nonce.map(|nonce| {
        let mut report_data = [0_u8; 64];
        report_data[..nonce.len()].copy_from_slice(&nonce);
        report_data
    });
    Ok(report_data)
}

fn require_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .as_text()
        .ok_or_else(|| format!("AWS Nitro {field} must be a CBOR text string"))
}

fn require_non_empty_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    let value = require_text(value, field)?;
    if value.is_empty() {
        return Err(format!("AWS Nitro {field} must not be empty"));
    }
    Ok(value)
}

fn require_bounded_bytes(
    value: &Value,
    field: &str,
    bounds: std::ops::RangeInclusive<usize>,
) -> Result<Vec<u8>, String> {
    let bytes = value
        .as_bytes()
        .ok_or_else(|| format!("AWS Nitro {field} must be a CBOR byte string"))?;
    if !bounds.contains(&bytes.len()) {
        return Err(format!(
            "AWS Nitro {field} length must be {}-{} bytes, got {}",
            bounds.start(),
            bounds.end(),
            bytes.len()
        ));
    }
    Ok(bytes.to_vec())
}

fn validate_nitro_pcrs(value: &Value) -> Result<std::collections::BTreeMap<u8, Vec<u8>>, String> {
    let entries = value
        .as_map()
        .ok_or_else(|| "AWS Nitro pcrs must be a CBOR map".to_string())?;
    if !(1..=32).contains(&entries.len()) {
        return Err(format!(
            "AWS Nitro pcrs must contain 1-32 entries, got {}",
            entries.len()
        ));
    }
    let mut pcrs = std::collections::BTreeMap::new();
    for (index, value) in entries {
        let index = index
            .as_integer()
            .and_then(|index| u8::try_from(index).ok())
            .filter(|index| *index < 32)
            .ok_or_else(|| "AWS Nitro PCR index must be an integer in 0-31".to_string())?;
        if pcrs.contains_key(&index) {
            return Err(format!("AWS Nitro pcrs contains duplicate index {index}"));
        }
        let value = value
            .as_bytes()
            .ok_or_else(|| format!("AWS Nitro PCR{index} must be a CBOR byte string"))?;
        if !matches!(value.len(), 32 | 48 | 64) {
            return Err(format!(
                "AWS Nitro PCR{index} must be 32, 48, or 64 bytes, got {}",
                value.len()
            ));
        }
        pcrs.insert(index, value.to_vec());
    }
    if pcrs.get(&0).is_none_or(|pcr| pcr.len() != 48) {
        return Err("AWS Nitro PCR0 must be a 48-byte SHA-384 measurement".to_string());
    }
    Ok(pcrs)
}

fn validate_nitro_certificate_usage(leaf_der: &[u8], cabundle: &[Vec<u8>]) -> Result<(), String> {
    let leaf = Certificate::from_der(leaf_der)
        .map_err(|e| format!("AWS Nitro leaf certificate is not valid X.509 DER: {e}"))?;
    let leaf_key_usage = leaf
        .tbs_certificate
        .get::<KeyUsage>()
        .map_err(|e| format!("AWS Nitro leaf key-usage extension is malformed: {e}"))?
        .ok_or_else(|| {
            "AWS Nitro leaf certificate is missing keyUsage (digitalSignature required)".to_string()
        })?;
    if !leaf_key_usage.1.digital_signature() {
        return Err("AWS Nitro leaf keyUsage does not permit digitalSignature".to_string());
    }
    if let Some((_, basic_constraints)) = leaf
        .tbs_certificate
        .get::<BasicConstraints>()
        .map_err(|e| format!("AWS Nitro leaf basic-constraints extension is malformed: {e}"))?
        && (basic_constraints.ca || basic_constraints.path_len_constraint.is_some())
    {
        return Err(
            "AWS Nitro leaf certificate must be an end entity without pathLenConstraint".into(),
        );
    }

    for (index, der) in cabundle.iter().enumerate() {
        let cert = Certificate::from_der(der)
            .map_err(|e| format!("AWS Nitro CA certificate {index} is not valid X.509 DER: {e}"))?;
        let basic_constraints = cert
            .tbs_certificate
            .get::<BasicConstraints>()
            .map_err(|e| {
                format!("AWS Nitro CA certificate {index} basic-constraints malformed: {e}")
            })?
            .ok_or_else(|| {
                format!("AWS Nitro CA certificate {index} is missing BasicConstraints")
            })?;
        if !basic_constraints.0 || !basic_constraints.1.ca {
            return Err(format!(
                "AWS Nitro CA certificate {index} must have critical BasicConstraints cA=true"
            ));
        }
        let key_usage = cert
            .tbs_certificate
            .get::<KeyUsage>()
            .map_err(|e| format!("AWS Nitro CA certificate {index} key-usage malformed: {e}"))?
            .ok_or_else(|| format!("AWS Nitro CA certificate {index} is missing keyUsage"))?;
        if !key_usage.0 {
            return Err(format!(
                "AWS Nitro CA certificate {index} keyUsage must be critical"
            ));
        }
        if !key_usage.1.key_cert_sign() {
            return Err(format!(
                "AWS Nitro CA certificate {index} keyUsage does not permit keyCertSign"
            ));
        }
    }
    let bundle_root = Certificate::from_der(&cabundle[0])
        .map_err(|e| format!("AWS Nitro CA bundle root is not valid X.509 DER: {e}"))?;
    if bundle_root.tbs_certificate.issuer != bundle_root.tbs_certificate.subject {
        return Err("AWS Nitro cabundle must start with a self-issued root certificate".into());
    }
    verify_nitro_root_self_signature(&bundle_root)?;
    Ok(())
}

fn verify_nitro_root_self_signature(root: &Certificate) -> Result<(), String> {
    use p384::ecdsa::{Signature, VerifyingKey, signature::Verifier};

    let public_key = root
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| "AWS Nitro CA bundle root public key is not octet-aligned".to_string())?;
    let verifying_key = VerifyingKey::from_sec1_bytes(public_key)
        .map_err(|e| format!("AWS Nitro CA bundle root key is not P-384: {e}"))?;
    let signature =
        Signature::from_der(root.signature.as_bytes().ok_or_else(|| {
            "AWS Nitro CA bundle root signature is not octet-aligned".to_string()
        })?)
        .map_err(|e| format!("AWS Nitro CA bundle root signature is not DER ECDSA: {e}"))?;
    let tbs = root
        .tbs_certificate
        .to_der()
        .map_err(|e| format!("AWS Nitro CA bundle root TBS certificate cannot be encoded: {e}"))?;
    verifying_key
        .verify(&tbs, &signature)
        .map_err(|e| format!("AWS Nitro CA bundle root is not self-signed: {e}"))
}
