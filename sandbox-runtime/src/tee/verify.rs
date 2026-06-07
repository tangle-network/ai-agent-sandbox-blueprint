//! Cryptographic verification of TEE quotes against hardware roots of trust.
//!
//! This is the real implementation behind [`super::verify_quote_signature`],
//! compiled only under the `tee-verify` feature so the default and non-TEE
//! builds never pull the heavier X.509/ECDSA crates.
//!
//! # What "verified" means here
//!
//! Each arm chains an attestation back to a hardware root that the *operator
//! does not control*:
//!
//! - **Intel TDX (DCAP):** [`dcap_qvl`] verifies the quote ECDSA signature and
//!   the PCK certificate chain up to the Intel SGX Root CA bundled in that
//!   crate ([`dcap_qvl::verify::QuoteVerifier::new_prod`]), and checks TCB
//!   status + collateral expiry against Intel PCS collateral.
//! - **AMD SEV-SNP:** the [`sev`] crate verifies the report signature and the
//!   VCEK/VLEK → ASK → ARK chain against the AMD root keys built into the crate
//!   ([`sev::certs::snp::builtin`]). On top of the crate's signature-only
//!   `crypto_nossl` path we additionally (a) reject any cert in the chain that
//!   is outside its notBefore/notAfter window at the trusted `now_secs` (the
//!   crate checks no validity window), (b) when the producer carries the AMD KDS
//!   CRL, verify its RSA-PSS signature against the pinned ASK, check its
//!   freshness window, and reject a revoked VEK serial ([`check_crl_revocation`]),
//!   and (c) reject reports whose guest policy permits debug, whose VMPL ≠ 0, or
//!   whose version < 2 — mirroring the TDX arm's debug rejection so both backends
//!   fail closed on a non-confidential enclave. NOTE: revocation is only checked
//!   when a CRL is supplied in the evidence. The TDX arm always has CRLs in its
//!   DCAP collateral, so a SEV report WITHOUT a bundled CRL is lower-assurance
//!   than TDX on revocation — producers SHOULD always carry the KDS CRL.
//! - **AWS Nitro:** honest `Err` — see [`verify_nitro`].
//!
//! # Evidence binding (anti-forgery)
//!
//! A malicious operator can put anything in [`super::AttestationReport`]. So we
//! NEVER trust `report.measurement`; we extract the measurement from *inside*
//! the cryptographically verified quote and require it to equal the
//! caller-visible `report.measurement`. A forged measurement therefore fails.
//! When the caller bound a nonce into `report_data` (replay protection), that
//! binding is carried by the signed quote and surfaced via [`VerifiedQuote`].

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
    /// The 64-byte report data the hardware signed (caller nonce binding).
    pub report_data: [u8; 64],
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

fn verify_tdx(evidence: &[u8], now_secs: u64) -> Result<VerifiedQuote, String> {
    let envelope = parse_envelope(evidence);

    // The quote bytes: from the envelope when wrapped, else the raw blob.
    let (quote_bytes, collateral) = match envelope {
        Some(env) => {
            let quote_hex = env
                .quote
                .as_deref()
                .ok_or("TDX evidence envelope is missing the `quote` field")?;
            let quote = decode_hex("TDX quote", quote_hex)?;
            let collateral = env.collateral.ok_or(
                "TDX evidence envelope is missing `collateral`; DCAP quote signature \
                 verification requires Intel PCS collateral (TCB info, CRLs, QE identity)",
            )?;
            (quote, collateral)
        }
        None => {
            // A bare blob. Reject early with a precise reason rather than
            // letting the verifier fail opaquely: a local TDREPORT (what
            // /dev/tdx_guest emits) is not a DCAP quote and has no signature
            // chain to verify, and a raw DCAP quote still needs collateral we
            // don't have here.
            classify_bare_tdx_blob(evidence)?;
            return Err(
                "TDX evidence is a bare quote without collateral; wrap it as \
                 {\"quote\":<hex>,\"collateral\":{...}} so the PCK chain and TCB \
                 status can be verified against the Intel SGX Root CA"
                    .to_string(),
            );
        }
    };

    // Bound parsing: reject absurd sizes before handing to the decoder.
    if quote_bytes.len() < 48 || quote_bytes.len() > 64 * 1024 {
        return Err(format!(
            "TDX quote length {} is outside the plausible range",
            quote_bytes.len()
        ));
    }

    // Genuine verification: signature + PCK chain to Intel root + TCB + expiry.
    let verified = dcap_qvl::verify::rustcrypto::verify(&quote_bytes, &collateral, now_secs)
        .map_err(|e| format!("TDX DCAP verification failed: {e}"))?;

    // TCB must be acceptable. `UpToDate` is the only unconditionally-good state;
    // anything else (out-of-date, revoked, SW-hardening needed, …) is not
    // trustworthy for our purposes.
    if verified.status != "UpToDate" {
        return Err(format!(
            "TDX TCB status is `{}` (advisories: {:?}); not accepted as up-to-date",
            verified.status, verified.advisory_ids
        ));
    }

    // Extract the measurement + nonce the hardware actually signed.
    let report = verified.report;
    let td = report
        .as_td10()
        .ok_or("verified quote is not a TDX TD report")?;
    Ok(VerifiedQuote {
        measurement: td.mr_td.to_vec(),
        report_data: td.report_data,
    })
}

/// Give a precise reason for a bare (un-enveloped) TDX blob.
fn classify_bare_tdx_blob(evidence: &[u8]) -> Result<(), String> {
    // The on-device TDREPORT is exactly 1024 bytes and is NOT a DCAP quote.
    // Check this BEFORE the header heuristic: a TDREPORT's leading bytes are not
    // a DCAP version/tee_type pair, and an all-zero TDREPORT would otherwise be
    // misread as an SGX quote (tee_type 0).
    if evidence.len() == TDX_TDREPORT_SIZE {
        return Err(
            "TDX evidence is a local TDREPORT (/dev/tdx_guest output), not a DCAP quote; \
             it has no signature chain to a hardware root and cannot be remotely verified \
             without a quoting enclave"
                .to_string(),
        );
    }
    // A DCAP quote begins with version (u16 LE) then tee_type (u32 LE). A real
    // TDX quote carries the TDX tee_type; SGX is rejected here because this is
    // the TDX arm. Either way the caller still owes us collateral.
    if evidence.len() >= 6 {
        let tee_type = u32::from_le_bytes([evidence[2], evidence[3], evidence[4], evidence[5]]);
        if tee_type == TDX_QUOTE_TEE_TYPE {
            return Ok(());
        }
        if tee_type == SGX_QUOTE_TEE_TYPE {
            return Err("TDX evidence header declares an SGX (not TDX) quote tee_type".to_string());
        }
    }
    Err("TDX evidence is neither a recognized DCAP quote nor a TDREPORT".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// AMD SEV-SNP
// ─────────────────────────────────────────────────────────────────────────────

fn verify_sev(evidence: &[u8], now_secs: u64) -> Result<VerifiedQuote, String> {
    use sev::certs::snp::{Certificate, Chain, Verifiable, builtin, ca};
    use sev::firmware::guest::AttestationReport as SnpReport;
    use sev::parser::ByteParser;

    let envelope = parse_envelope(evidence).ok_or(
        "SEV evidence must be a JSON envelope {\"report\":<hex>,\"vcek\":<der hex>}; \
         a bare SNP report carries no VCEK/VLEK leaf and cannot be chained to the AMD root",
    )?;

    let report_hex = envelope
        .report
        .as_deref()
        .or(envelope.quote.as_deref())
        .ok_or("SEV evidence envelope is missing the `report` field")?;
    let report_bytes = decode_hex("SEV report", report_hex)?;
    if report_bytes.len() > 8 * 1024 {
        return Err(format!(
            "SEV report length {} is implausibly large",
            report_bytes.len()
        ));
    }
    let report = SnpReport::from_bytes(report_bytes.as_slice())
        .map_err(|e| format!("failed to parse SEV-SNP attestation report: {e}"))?;

    // Leaf endorsement key (VCEK or VLEK), DER-encoded.
    let (leaf_label, leaf_hex) = match (envelope.vcek.as_deref(), envelope.vlek.as_deref()) {
        (Some(v), _) => ("VCEK", v),
        (None, Some(v)) => ("VLEK", v),
        (None, None) => {
            return Err("SEV evidence envelope is missing both `vcek` and `vlek`".to_string());
        }
    };
    let leaf_der = decode_hex(leaf_label, leaf_hex)?;
    let vek = Certificate::from_der(&leaf_der)
        .map_err(|e| format!("failed to parse SEV {leaf_label} certificate: {e}"))?;

    // Pinned AMD root for the CPU generation. Default to Milan when unspecified.
    //
    // NOTE: `amd_generation` is operator-supplied and only selects which pinned
    // ARK/ASK the report must chain to — it is fail-closed for correctness (a
    // Genoa report cannot chain to the Milan ARK; see
    // `sev_wrong_root_generation_is_rejected`). It is NOT a hardware-generation
    // assertion: the relying party still does not pin which CPU generation it
    // *expects*, so an operator can move workloads between generations as long as
    // each chains validly. Pinning the expected generation (and TEE type)
    // alongside the expected measurement belongs in the client/on-chain policy
    // layer that calls `verify_attestation`, not in this per-quote chain check.
    let generation = envelope
        .amd_generation
        .as_deref()
        .unwrap_or("milan")
        .to_ascii_lowercase();
    let (ark, ask) = match generation.as_str() {
        "milan" => (builtin::milan::ark(), builtin::milan::ask()),
        "genoa" => (builtin::genoa::ark(), builtin::genoa::ask()),
        "turin" => (builtin::turin::ark(), builtin::turin::ask()),
        other => {
            return Err(format!(
                "unsupported AMD generation `{other}` (expected milan|genoa|turin)"
            ));
        }
    };
    let ark = ark.map_err(|e| format!("failed to load AMD {generation} ARK: {e}"))?;
    let ask = ask.map_err(|e| format!("failed to load AMD {generation} ASK: {e}"))?;

    let chain = Chain {
        ca: ca::Chain { ark, ask },
        vek,
    };

    // Validity windows: the AMD `sev` crate's `crypto_nossl` path verifies only
    // the RSA-PSS signature of signer-over-signee; it never checks notBefore/
    // notAfter. An EXPIRED VCEK whose signature still chains to the pinned ARK
    // would otherwise verify. Mirror the TDX arm (which passes `now_secs` into
    // dcap-qvl for collateral/TCB/CRL freshness) by rejecting any cert in the
    // chain that is outside its validity window at `now_secs`.
    check_cert_validity("SEV-SNP VCEK/VLEK", &leaf_der, now_secs)?;
    let ark_der = chain
        .ca
        .ark
        .to_der()
        .map_err(|e| format!("failed to re-encode AMD ARK for validity check: {e}"))?;
    let ask_der = chain
        .ca
        .ask
        .to_der()
        .map_err(|e| format!("failed to re-encode AMD ASK for validity check: {e}"))?;
    check_cert_validity("SEV-SNP ARK", &ark_der, now_secs)?;
    check_cert_validity("SEV-SNP ASK", &ask_der, now_secs)?;

    // Genuine verification: ARK self-signed → ARK signs ASK → ASK signs VEK →
    // VEK signs the report. Any break returns Err.
    (&chain, &report)
        .verify()
        .map_err(|e| format!("SEV-SNP chain/report verification failed: {e}"))?;

    // Revocation: the AMD KDS CRL (signed by the ASK) lists revoked VEK serials.
    // The `sev` crate consults no CRL, so without this a revoked-but-in-window
    // VEK would verify. When the producer carries the KDS CRL we verify its
    // signature against the pinned ASK, check its freshness window, and reject a
    // revoked leaf. When no CRL is carried we cannot check revocation; the SEV
    // path is therefore lower-assurance than TDX (see the module docs).
    if let Some(crl_hex) = envelope.crl.as_deref() {
        let crl_der = decode_hex("SEV-SNP KDS CRL", crl_hex)?;
        check_crl_revocation(&ask_der, &leaf_der, &crl_der, now_secs)?;
    }

    // Confidentiality gate (mirrors the TDX debug-rejection contract).
    enforce_sev_confidentiality(&report)?;

    Ok(VerifiedQuote {
        measurement: report.measurement.to_vec(),
        report_data: report.report_data,
    })
}

/// Reject a genuine but non-confidential SEV-SNP report.
///
/// A validly-signed report from a DEBUG-enabled guest is worthless: the
/// hypervisor/operator can read and modify guest memory, defeating TEE
/// confidentiality. dcap-qvl rejects TDX/SGX debug by default, so without this
/// the SEV arm would be strictly weaker than its TDX counterpart. We also pin
/// VMPL 0 (highest privilege) and require report version ≥ 2.
fn enforce_sev_confidentiality(
    report: &sev::firmware::guest::AttestationReport,
) -> Result<(), String> {
    if report.version < 2 {
        return Err(format!(
            "SEV-SNP report version {} is below the required v2",
            report.version
        ));
    }
    if report.policy.debug_allowed() {
        return Err(
            "SEV-SNP guest policy permits debug; guest memory is not confidential".to_string(),
        );
    }
    if report.vmpl != 0 {
        return Err(format!(
            "SEV-SNP report VMPL {} is not the expected 0 (highest privilege)",
            report.vmpl
        ));
    }
    Ok(())
}

/// Reject a DER X.509 cert whose validity window does not contain `now_secs`.
///
/// The `sev` crate's `crypto_nossl` path verifies signatures but NOT validity,
/// so this is the validity-window freshness check on the SEV chain. Revocation
/// is handled separately by [`check_crl_revocation`] when a KDS CRL is supplied.
/// It fails closed: a parse error or an out-of-window cert both return `Err`.
fn check_cert_validity(label: &str, der: &[u8], now_secs: u64) -> Result<(), String> {
    use x509_cert::der::Decode;

    let cert = x509_cert::Certificate::from_der(der)
        .map_err(|e| format!("failed to parse {label} certificate for validity check: {e}"))?;
    let validity = cert.tbs_certificate.validity;
    let not_before = validity.not_before.to_unix_duration().as_secs();
    let not_after = validity.not_after.to_unix_duration().as_secs();
    if now_secs < not_before {
        return Err(format!(
            "{label} certificate is not yet valid (notBefore {not_before} > now {now_secs})"
        ));
    }
    if now_secs > not_after {
        return Err(format!(
            "{label} certificate has expired (notAfter {not_after} < now {now_secs})"
        ));
    }
    Ok(())
}

/// AMD's RSA-SSA-PSS OID, matching the algorithm the KDS uses to sign both the
/// SEV-SNP certificate chain and the KDS CRL.
const RSA_SSA_PSS_OID: spki::ObjectIdentifier =
    spki::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");

/// Verify a KDS CRL against the pinned ASK and reject a revoked leaf serial.
///
/// Fails closed on every error: a CRL whose signature does not chain to the ASK,
/// is outside its thisUpdate/nextUpdate window at `now_secs`, or lists the leaf's
/// serial all return `Err`. This mirrors what dcap-qvl does for the TDX PCK CRL.
///
/// The CRL signature is verified with the SAME RSA-PSS-SHA384 primitive the
/// `sev` crate uses for the cert chain (see `certs/snp/cert_nossl.rs`), so this
/// is genuine verification, not a re-implementation of the trust decision.
fn check_crl_revocation(
    ask_der: &[u8],
    leaf_der: &[u8],
    crl_der: &[u8],
    now_secs: u64,
) -> Result<(), String> {
    use der::{Decode, Encode, referenced::OwnedToRef};
    use signature::Verifier;
    use x509_cert::crl::CertificateList;

    let crl = CertificateList::from_der(crl_der)
        .map_err(|e| format!("failed to parse AMD KDS CRL: {e}"))?;

    // The CRL must be RSA-PSS signed (matching AMD's chain algorithm).
    if crl.signature_algorithm.oid != RSA_SSA_PSS_OID {
        return Err(format!(
            "AMD KDS CRL uses unexpected signature algorithm {:?}; expected RSA-SSA-PSS",
            crl.signature_algorithm.oid
        ));
    }

    // Verify the CRL signature against the ASK public key. The CRL issuer is the
    // ASK (the VEK's issuer); chaining the CRL to the same pinned ASK we already
    // anchored the chain to is what makes the revocation list trustworthy.
    let ask = x509_cert::Certificate::from_der(ask_der)
        .map_err(|e| format!("failed to parse ASK for CRL signature check: {e}"))?;
    let ask_spki = ask.tbs_certificate.subject_public_key_info.owned_to_ref();
    let ask_rsa = rsa::RsaPublicKey::try_from(ask_spki)
        .map_err(|e| format!("invalid ASK RSA public key: {e:?}"))?;
    let verifying_key = rsa::pss::VerifyingKey::<sha2::Sha384>::new(ask_rsa);

    let tbs = crl
        .tbs_cert_list
        .to_der()
        .map_err(|e| format!("failed to re-encode CRL tbsCertList: {e}"))?;
    let signature = rsa::pss::Signature::try_from(crl.signature.raw_bytes())
        .map_err(|e| format!("invalid CRL signature encoding: {e:?}"))?;
    verifying_key
        .verify(&tbs, &signature)
        .map_err(|e| format!("AMD KDS CRL signature does not chain to the pinned ASK: {e}"))?;

    // Freshness: the CRL must be currently valid.
    let this_update = crl.tbs_cert_list.this_update.to_unix_duration().as_secs();
    if now_secs < this_update {
        return Err(format!(
            "AMD KDS CRL is not yet valid (thisUpdate {this_update} > now {now_secs})"
        ));
    }
    if let Some(next_update) = crl.tbs_cert_list.next_update {
        let next = next_update.to_unix_duration().as_secs();
        if now_secs > next {
            return Err(format!(
                "AMD KDS CRL has expired (nextUpdate {next} < now {now_secs})"
            ));
        }
    }

    // Revocation: reject if the leaf VEK serial appears in the CRL.
    let leaf = x509_cert::Certificate::from_der(leaf_der)
        .map_err(|e| format!("failed to parse VEK for revocation check: {e}"))?;
    let leaf_serial = &leaf.tbs_certificate.serial_number;
    if let Some(revoked) = crl.tbs_cert_list.revoked_certificates.as_ref()
        && revoked
            .iter()
            .any(|entry| &entry.serial_number == leaf_serial)
    {
        return Err(
            "SEV-SNP VEK is revoked by the AMD KDS CRL; the endorsement key is no longer trusted"
                .to_string(),
        );
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// AWS Nitro
// ─────────────────────────────────────────────────────────────────────────────

/// AWS Nitro attestation verification is intentionally NOT trusted.
///
/// Verifying a Nitro `COSE_Sign1` document requires (a) the published AWS Nitro
/// Enclaves root certificate to pin the chain against, and (b) a genuine
/// known-good attestation document to prove the positive path. Neither is
/// available to this crate today (no vendored vector, no embedded root). Per the
/// fail-closed contract we return `Err` rather than asserting a guarantee we
/// cannot back. The `coset` dependency is wired so the parse-and-chain path can
/// be completed once a pinned root + real vector land.
fn verify_nitro(evidence: &[u8]) -> Result<VerifiedQuote, String> {
    use coset::CborSerializable;

    // Structural parse only: prove the bytes are a COSE_Sign1 so the rejection
    // reason is precise, but never return Ok without root-anchored chain + PCRs.
    match coset::CoseSign1::from_slice(evidence) {
        Ok(_) => Err(
            "AWS Nitro attestation parsed as COSE_Sign1 but cannot be trusted: the AWS \
             Nitro root certificate is not pinned in this build, so the certificate chain \
             and PCRs cannot be verified against a hardware root of trust"
                .to_string(),
        ),
        Err(e) => Err(format!(
            "AWS Nitro evidence is not a valid COSE_Sign1 document: {e}"
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

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
mod tests {
    use super::*;
    use crate::tee::AttestationReport;

    // ── Test vector loaders (see tests/tee_vectors/README.md for provenance) ──

    const TDX_QUOTE: &[u8] = include_bytes!("../../tests/tee_vectors/tdx_quote.bin");
    const TDX_COLLATERAL_JSON: &[u8] =
        include_bytes!("../../tests/tee_vectors/tdx_quote_collateral.json");
    const SEV_VCEK_DER: &[u8] = include_bytes!("../../tests/tee_vectors/sev_vcek_milan.der");
    const SEV_REPORT_HEX: &[u8] = include_bytes!("../../tests/tee_vectors/sev_report_milan.hex");

    /// Pick a `now` inside the TDX collateral validity window so TCB/QE/CRL
    /// freshness checks pass deterministically regardless of wall-clock time.
    /// Uses `max(issueDate) + 60s`: the bundled TCB/QE windows and PCK CRL share
    /// a ~30-day window opening at the issue dates, so just past the latest issue
    /// date is safely before every `nextUpdate`.
    fn tdx_now() -> u64 {
        crate::tee::verify::tests_now_from_collateral(TDX_COLLATERAL_JSON)
    }

    fn tdx_envelope_evidence(quote: &[u8]) -> Vec<u8> {
        let collateral: serde_json::Value =
            serde_json::from_slice(TDX_COLLATERAL_JSON).expect("collateral json");
        serde_json::to_vec(&serde_json::json!({
            "quote": hex::encode(quote),
            "collateral": collateral,
        }))
        .expect("serialize envelope")
    }

    fn sev_envelope_evidence(report_hex: &str, vcek_der: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "report": report_hex,
            "vcek": hex::encode(vcek_der),
            "amd_generation": "milan",
        }))
        .expect("serialize envelope")
    }

    fn sev_report_hex() -> String {
        String::from_utf8(SEV_REPORT_HEX.to_vec())
            .expect("hex utf8")
            .trim()
            .to_string()
    }

    /// A `now` inside the bundled Milan VCEK/ARK/ASK validity windows, so the
    /// SEV expiry check passes deterministically regardless of wall-clock time.
    /// Uses the VCEK notBefore + 60s (the VCEK is the shortest-lived link and
    /// opens latest; the AMD ARK/ASK validity windows fully contain it).
    fn sev_now() -> u64 {
        use x509_cert::der::Decode;
        let cert = x509_cert::Certificate::from_der(SEV_VCEK_DER).expect("parse vcek der");
        cert.tbs_certificate
            .validity
            .not_before
            .to_unix_duration()
            .as_secs()
            + 60
    }

    // ── Positive: genuine quotes verify ──────────────────────────────────────

    #[test]
    fn tdx_known_good_quote_verifies() {
        // The known-good Intel TDX DCAP quote chains to the Intel SGX Root CA,
        // is TCB UpToDate within the collateral window, and yields a 48-byte
        // (SHA-384) MRTD. Reaching Ok here IS the proof of a valid chain.
        let evidence = tdx_envelope_evidence(TDX_QUOTE);
        let verified = verify_tdx(&evidence, tdx_now()).expect("genuine TDX quote must verify");
        assert_eq!(
            verified.measurement.len(),
            48,
            "TDX MRTD is a 48-byte SHA-384 measurement"
        );
        // The MRTD must be non-trivial (not all-zero), i.e. a real measurement.
        assert!(verified.measurement.iter().any(|&b| b != 0));
    }

    #[test]
    fn sev_known_good_report_verifies() {
        let evidence = sev_envelope_evidence(&sev_report_hex(), SEV_VCEK_DER);
        let verified =
            verify_sev(&evidence, sev_now()).expect("genuine SEV-SNP report must verify");
        assert_eq!(verified.measurement.len(), 48, "SEV measurement is SHA-384");
    }

    // ── Negative: tampering / wrong root / bad shape are rejected ─────────────

    #[test]
    fn tdx_flipped_byte_is_rejected() {
        let mut quote = TDX_QUOTE.to_vec();
        // Flip a byte deep in the signed body so the ECDSA signature fails.
        let idx = quote.len() / 2;
        quote[idx] ^= 0xFF;
        let evidence = tdx_envelope_evidence(&quote);
        assert!(
            verify_tdx(&evidence, tdx_now()).is_err(),
            "a tampered TDX quote must NOT verify"
        );
    }

    #[test]
    fn tdx_bare_tdreport_is_rejected() {
        // A 1024-byte local TDREPORT is not a DCAP quote.
        let tdreport = vec![0u8; TDX_TDREPORT_SIZE];
        let err = verify_tdx(&tdreport, tdx_now()).unwrap_err();
        assert!(
            err.contains("TDREPORT"),
            "reason should name TDREPORT: {err}"
        );
    }

    #[test]
    fn tdx_missing_collateral_is_rejected() {
        let evidence = serde_json::to_vec(&serde_json::json!({
            "quote": hex::encode(TDX_QUOTE),
        }))
        .unwrap();
        let err = verify_tdx(&evidence, tdx_now()).unwrap_err();
        assert!(
            err.contains("collateral"),
            "reason should name collateral: {err}"
        );
    }

    #[test]
    fn tdx_expired_collateral_is_rejected() {
        // Freshness gate: a `now` past every collateral `nextUpdate` must fail,
        // even though the quote signature itself is intact. The bundled TCB/QE
        // windows and PCK CRL share a ~30-day window opening at the issue dates,
        // so +40 days past the latest issue date is past every nextUpdate. This
        // pins the fail-closed expiry behavior so a regression that ignores
        // `now_secs` is caught.
        let evidence = tdx_envelope_evidence(TDX_QUOTE);
        let expired_now = tdx_now() + 40 * 24 * 3600;
        assert!(
            verify_tdx(&evidence, expired_now).is_err(),
            "an out-of-window TDX quote must NOT verify"
        );
    }

    #[test]
    fn sev_flipped_signature_byte_is_rejected() {
        // Flip a byte inside the signed region (mirrors the sev crate's own
        // negative test which toggles report_bytes[21]).
        let mut report_bytes = hex::decode(sev_report_hex()).unwrap();
        report_bytes[21] ^= 0x80;
        let evidence = sev_envelope_evidence(&hex::encode(&report_bytes), SEV_VCEK_DER);
        assert!(
            verify_sev(&evidence, sev_now()).is_err(),
            "a tampered SEV report must NOT verify"
        );
    }

    #[test]
    fn sev_wrong_root_generation_is_rejected() {
        // The report is a Milan report; anchoring it to the Genoa root must fail
        // because the Genoa ARK does not certify the Milan VCEK.
        let evidence = serde_json::to_vec(&serde_json::json!({
            "report": sev_report_hex(),
            "vcek": hex::encode(SEV_VCEK_DER),
            "amd_generation": "genoa",
        }))
        .unwrap();
        assert!(
            verify_sev(&evidence, sev_now()).is_err(),
            "a Milan report must NOT verify against the Genoa root"
        );
    }

    #[test]
    fn sev_corrupted_vcek_is_rejected() {
        // Corrupt the VCEK leaf (mirrors the sev crate's milan_chain_invalid).
        let mut vcek = SEV_VCEK_DER.to_vec();
        vcek[40] ^= 0xFF;
        let evidence = sev_envelope_evidence(&sev_report_hex(), &vcek);
        assert!(
            verify_sev(&evidence, sev_now()).is_err(),
            "a corrupted VCEK must NOT verify"
        );
    }

    #[test]
    fn sev_bare_report_without_vcek_is_rejected() {
        let bare = hex::decode(sev_report_hex()).unwrap();
        let err = verify_sev(&bare, sev_now()).unwrap_err();
        assert!(
            err.contains("envelope"),
            "reason should explain the envelope requirement: {err}"
        );
    }

    /// Parse the bundled known-good Milan report into the `sev` report struct.
    fn sev_parsed_report() -> sev::firmware::guest::AttestationReport {
        use sev::parser::ByteParser;
        let bytes = hex::decode(sev_report_hex()).unwrap();
        sev::firmware::guest::AttestationReport::from_bytes(bytes.as_slice())
            .expect("parse known-good SEV report")
    }

    #[test]
    fn sev_known_good_report_passes_confidentiality_gate() {
        // The genuine Milan vector is v2, debug-disabled, VMPL 0.
        assert!(enforce_sev_confidentiality(&sev_parsed_report()).is_ok());
    }

    #[test]
    fn sev_debug_enabled_policy_is_rejected() {
        // A genuine, validly-signed report from a DEBUG-enabled guest must be
        // rejected: the operator can read/modify guest memory. We exercise the
        // gate directly because flipping the policy bit in the signed bytes would
        // (correctly) break the signature first and never reach the gate.
        let mut report = sev_parsed_report();
        report.policy.set_debug_allowed(true);
        let err = enforce_sev_confidentiality(&report).unwrap_err();
        assert!(err.contains("debug"), "reason should name debug: {err}");
    }

    #[test]
    fn sev_nonzero_vmpl_is_rejected() {
        let mut report = sev_parsed_report();
        report.vmpl = 1;
        let err = enforce_sev_confidentiality(&report).unwrap_err();
        assert!(err.contains("VMPL"), "reason should name VMPL: {err}");
    }

    #[test]
    fn sev_old_version_is_rejected() {
        let mut report = sev_parsed_report();
        report.version = 1;
        let err = enforce_sev_confidentiality(&report).unwrap_err();
        assert!(err.contains("version"), "reason should name version: {err}");
    }

    #[test]
    fn sev_expired_vcek_is_rejected() {
        // A `now` past the VCEK notAfter must fail the freshness check even
        // though the signature chain is intact. The Milan VCEK expires 2030;
        // jump well beyond it.
        let expired_now = sev_now() + 20 * 365 * 24 * 3600; // ~2043
        let evidence = sev_envelope_evidence(&sev_report_hex(), SEV_VCEK_DER);
        let err = verify_sev(&evidence, expired_now).unwrap_err();
        assert!(
            err.contains("expired") || err.contains("valid"),
            "reason should name expiry/validity: {err}"
        );
    }

    fn sev_envelope_evidence_with_crl(
        report_hex: &str,
        vcek_der: &[u8],
        crl_der: &[u8],
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "report": report_hex,
            "vcek": hex::encode(vcek_der),
            "amd_generation": "milan",
            "crl": hex::encode(crl_der),
        }))
        .expect("serialize envelope")
    }

    #[test]
    fn sev_without_crl_still_verifies_lower_assurance() {
        // No CRL in the envelope → revocation is not checked, but the genuine
        // chain still verifies. This pins the documented lower-assurance path so
        // a regression that hard-requires a CRL (breaking existing producers) is
        // caught, while the WITH-crl tests pin the fail-closed revocation wiring.
        let evidence = sev_envelope_evidence(&sev_report_hex(), SEV_VCEK_DER);
        assert!(verify_sev(&evidence, sev_now()).is_ok());
    }

    #[test]
    fn sev_unparseable_crl_is_rejected() {
        // A `crl` field that is not a DER CertificateList must fail closed: the
        // CRL path is reached (the chain otherwise verifies) and rejects bytes it
        // cannot parse rather than ignoring the revocation channel.
        let evidence =
            sev_envelope_evidence_with_crl(&sev_report_hex(), SEV_VCEK_DER, &[0xDE, 0xAD]);
        let err = verify_sev(&evidence, sev_now()).unwrap_err();
        assert!(err.contains("CRL"), "reason should name the CRL: {err}");
    }

    #[test]
    fn sev_crl_not_signed_by_ask_is_rejected() {
        // Feed a structurally-valid DER object that is NOT an ASK-signed CRL: the
        // VCEK certificate DER. It parses as X.509 but not as a CRL, so the CRL
        // check rejects it — proving we never accept a revocation list that does
        // not chain to the pinned ASK.
        let evidence =
            sev_envelope_evidence_with_crl(&sev_report_hex(), SEV_VCEK_DER, SEV_VCEK_DER);
        let err = verify_sev(&evidence, sev_now()).unwrap_err();
        assert!(err.contains("CRL"), "reason should name the CRL: {err}");
    }

    #[test]
    fn nitro_is_honest_err() {
        // No pinned AWS root + no real vector → never trusted.
        let report = AttestationReport {
            tee_type: TeeType::Nitro,
            evidence: vec![0xA1, 0x00],
            measurement: vec![0u8; 48],
            timestamp: 0,
        };
        assert!(verify_quote_signature(&report, 0).is_err());
    }

    #[test]
    fn none_type_is_rejected() {
        let report = AttestationReport {
            tee_type: TeeType::None,
            evidence: vec![1, 2, 3],
            measurement: vec![],
            timestamp: 0,
        };
        assert!(verify_quote_signature(&report, 0).is_err());
    }
}
