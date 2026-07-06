//! tee/verify unit tests.

use super::*;

#[cfg(test)]
mod cases {
    use super::*;
    use crate::tee::AttestationReport;

    // ── Test vector loaders (see tests/tee_vectors/README.md for provenance) ──

    const TDX_QUOTE: &[u8] = include_bytes!("../../../tests/tee_vectors/tdx_quote.bin");
    const TDX_COLLATERAL_JSON: &[u8] =
        include_bytes!("../../../tests/tee_vectors/tdx_quote_collateral.json");
    const SEV_VCEK_DER: &[u8] = include_bytes!("../../../tests/tee_vectors/sev_vcek_milan.der");
    const SEV_REPORT_HEX: &[u8] = include_bytes!("../../../tests/tee_vectors/sev_report_milan.hex");

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
        // Past every bundled TCB/QE/CRL nextUpdate (each a ~30-day window).
        const PAST_ALL_NEXTUPDATE_SECS: u64 = 40 * 24 * 3600;
        let expired_now = tdx_now() + PAST_ALL_NEXTUPDATE_SECS;
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
