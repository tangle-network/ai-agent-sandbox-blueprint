//! Intel TDX quote verification.

use super::*;

pub(crate) fn verify_tdx(evidence: &[u8], now_secs: u64) -> Result<VerifiedQuote, String> {
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
pub(crate) fn classify_bare_tdx_blob(evidence: &[u8]) -> Result<(), String> {
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
