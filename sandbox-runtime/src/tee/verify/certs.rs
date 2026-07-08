//! X.509 certificate validity + CRL revocation checks.

/// Reject a DER X.509 cert whose validity window does not contain `now_secs`.
///
/// The `sev` crate's `crypto_nossl` path verifies signatures but NOT validity,
/// so this is the validity-window freshness check on the SEV chain. Revocation
/// is handled separately by [`check_crl_revocation`] when a KDS CRL is supplied.
/// It fails closed: a parse error or an out-of-window cert both return `Err`.
pub(crate) fn check_cert_validity(label: &str, der: &[u8], now_secs: u64) -> Result<(), String> {
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
pub(crate) const RSA_SSA_PSS_OID: spki::ObjectIdentifier =
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
pub(crate) fn check_crl_revocation(
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
