//! AMD SEV-SNP report verification.

use super::*;

pub(crate) fn verify_sev(evidence: &[u8], now_secs: u64) -> Result<VerifiedQuote, String> {
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
pub(crate) fn enforce_sev_confidentiality(
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
