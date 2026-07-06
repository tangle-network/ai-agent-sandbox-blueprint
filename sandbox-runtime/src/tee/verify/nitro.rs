//! AWS Nitro attestation-document verification.

use super::*;

/// AWS Nitro attestation verification is intentionally NOT trusted.
///
/// Verifying a Nitro `COSE_Sign1` document requires (a) the published AWS Nitro
/// Enclaves root certificate to pin the chain against, and (b) a genuine
/// known-good attestation document to prove the positive path. Neither is
/// available to this crate today (no vendored vector, no embedded root). Per the
/// fail-closed contract we return `Err` rather than asserting a guarantee we
/// cannot back. The `coset` dependency is wired so the parse-and-chain path can
/// be completed once a pinned root + real vector land.
pub(crate) fn verify_nitro(evidence: &[u8]) -> Result<VerifiedQuote, String> {
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
