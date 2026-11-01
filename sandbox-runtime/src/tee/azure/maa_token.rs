//! Verify a Microsoft Azure Attestation token.
//!
//! The Azure confidential-VM sidecar sends the hardware report to Microsoft
//! Azure Attestation and receives an RS256 JWT. This check verifies that
//! signature against keys published by the token's issuer host, then requires
//! the production confidential-VM claims. It does not re-verify the SNP report
//! embedded in the signing certificate, and it does not trust the JWT `jku`
//! header. A customer-owned `*.attest.azure.net` provider is signed by
//! Microsoft but can run its own policy; this is not a hardware-root check of
//! a raw quote. Evidence that is not a JWT is left for the SEV quote verifier.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::sha2::Sha256;
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};

use crate::error::{Result, SandboxError};
use crate::tee::AttestationReport;

const HOST_SUFFIX: &str = ".attest.azure.net";
const MAX_TOKEN_BYTES: usize = 8192;

pub(crate) enum MaaDecision {
    Verified,
    NotAJwt,
}

#[derive(serde::Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(serde::Deserialize)]
struct Jwk {
    #[serde(default)]
    kid: String,
    #[serde(default)]
    n: String,
    #[serde(default)]
    e: String,
    #[serde(default)]
    kty: Option<String>,
    #[serde(default)]
    alg: Option<String>,
}

#[derive(serde::Deserialize)]
struct Header {
    alg: String,
    kid: String,
    #[serde(default)]
    crit: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct TokenClaims {
    iss: String,
    exp: u64,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default, rename = "rp_data")]
    rp_data: Option<String>,
    #[serde(rename = "x-ms-ver")]
    x_ms_ver: String,
    #[serde(rename = "x-ms-attestation-type")]
    attestation_type: String,
    #[serde(rename = "x-ms-compliance-status")]
    compliance: String,
    #[serde(default, rename = "x-ms-sevsnpvm-is-debuggable")]
    sev_debuggable: Option<bool>,
    #[serde(default)]
    dbgstat: Option<String>,
    #[serde(default, rename = "tdx_td_attributes_debug")]
    tdx_debug: Option<bool>,
    #[serde(default, rename = "x-ms-runtime")]
    runtime: Option<RuntimeClaims>,
}

#[derive(serde::Deserialize)]
struct RuntimeClaims {
    #[serde(default, rename = "vm-configuration")]
    vm_configuration: Option<VmConfiguration>,
}

#[derive(serde::Deserialize)]
struct VmConfiguration {
    #[serde(default, rename = "secure-boot")]
    secure_boot: Option<bool>,
}

pub(crate) async fn require_production_token(
    report: &AttestationReport,
    expected_nonce: Option<&[u8; 64]>,
) -> Result<MaaDecision> {
    if report.evidence.len() > MAX_TOKEN_BYTES {
        if jwt_shaped(&report.evidence) {
            return Err(SandboxError::CloudProvider(
                "Azure attestation evidence is too large".into(),
            ));
        }
        return Ok(MaaDecision::NotAJwt);
    }
    let Some(token) = compact_jwt(&report.evidence) else {
        return Ok(MaaDecision::NotAJwt);
    };
    let header = token_header(&token)?;
    if header.alg != "RS256" {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token must use RS256".into(),
        ));
    }
    if header.crit.as_ref().is_some_and(|crit| !crit.is_empty()) {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token has an unsupported critical header".into(),
        ));
    }
    let claims = token_claims(&token)?;
    let origin = issuer_origin(&claims.iss)?;
    let jwk = signing_key(&origin, &header.kid).await?;
    verify_rs256(&token, &jwk)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SandboxError::CloudProvider("system clock is before the unix epoch".into()))?
        .as_secs();
    if claims.exp.saturating_add(60) < now {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token is expired".into(),
        ));
    }
    if claims.nbf.is_some_and(|nbf| nbf > now + 60) {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token is not valid yet".into(),
        ));
    }
    if issuer_origin(&claims.iss)? != origin {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token issuer was not the key host".into(),
        ));
    }
    production_claims(&claims)?;
    if let Some(vm) = claims
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.vm_configuration.as_ref())
        && vm.secure_boot != Some(true)
    {
        return Err(SandboxError::CloudProvider(
            "Azure attestation secure boot is not enabled".into(),
        ));
    }
    if let Some(expected) = expected_nonce
        && !nonce_matches(&claims, expected)
    {
        return Err(SandboxError::CloudProvider(
            "Azure attestation nonce did not match the caller challenge".into(),
        ));
    }
    Ok(MaaDecision::Verified)
}

fn production_claims(claims: &TokenClaims) -> Result<()> {
    if claims.x_ms_ver != "1.0" {
        return Err(SandboxError::CloudProvider(
            "Azure attestation token version is not 1.0".into(),
        ));
    }
    if claims.compliance != "azure-compliant-cvm" {
        return Err(SandboxError::CloudProvider(
            "Azure attestation is not an Azure-compliant confidential VM".into(),
        ));
    }
    match claims.attestation_type.as_str() {
        "sevsnpvm" => {
            if claims.sev_debuggable != Some(false) {
                return Err(SandboxError::CloudProvider(
                    "Azure SEV-SNP attestation is debuggable".into(),
                ));
            }
        }
        "tdxvm" => {
            if claims.dbgstat.as_deref() != Some("disabled") || claims.tdx_debug != Some(false) {
                return Err(SandboxError::CloudProvider(
                    "Azure TDX attestation is debuggable".into(),
                ));
            }
        }
        _ => {
            return Err(SandboxError::CloudProvider(
                "Azure attestation is not a confidential VM token".into(),
            ));
        }
    }
    Ok(())
}

fn issuer_origin(issuer: &str) -> Result<String> {
    let host = issuer.strip_prefix("https://").ok_or_else(|| {
        SandboxError::CloudProvider("Azure attestation issuer was not Azure Attestation".into())
    })?;
    if host.contains(['@', '/', '?', '#', ':'])
        || !host.ends_with(HOST_SUFFIX)
        || host.len() == HOST_SUFFIX.len()
        || host.split('.').any(|label| label.is_empty())
    {
        return Err(SandboxError::CloudProvider(
            "Azure attestation issuer was not Azure Attestation".into(),
        ));
    }
    Ok(format!("https://{host}"))
}

fn nonce_matches(claims: &TokenClaims, expected: &[u8; 64]) -> bool {
    let Some(presented) = claims.nonce.as_deref().or(claims.rp_data.as_deref()) else {
        return false;
    };
    let presented = presented.trim().trim_start_matches("0x");
    let want = hex::encode(expected);
    if presented.len() != want.len() {
        return false;
    }
    let folded = presented.to_ascii_lowercase();
    bool::from(subtle::ConstantTimeEq::ct_eq(
        folded.as_bytes(),
        want.as_bytes(),
    ))
}

fn jwt_shaped(evidence: &[u8]) -> bool {
    let Ok(raw) = std::str::from_utf8(evidence) else {
        return false;
    };
    let trimmed = raw.trim();
    let body = unchunk(trimmed).unwrap_or(trimmed);
    let mut compact = body.chars().filter(|c| !c.is_ascii_whitespace());
    compact.next() == Some('e') && compact.next() == Some('y') && compact.next() == Some('J')
}

fn compact_jwt(evidence: &[u8]) -> Option<String> {
    let raw = std::str::from_utf8(evidence).ok()?;
    let body = unchunk(raw.trim()).unwrap_or(raw.trim());
    let compact: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !compact.starts_with("eyJ") || compact.split('.').count() != 3 {
        return None;
    }
    Some(compact)
}

fn unchunk(raw: &str) -> Option<&str> {
    let (size, rest) = raw.split_once("\r\n")?;
    if size.is_empty() || !size.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let len = usize::from_str_radix(size, 16).ok()?;
    rest.get(..len)
}

fn token_header(token: &str) -> Result<Header> {
    decode_json(
        token.split('.').next().ok_or_else(|| {
            SandboxError::CloudProvider("Azure attestation evidence is not a JWT".into())
        })?,
        "header",
    )
}

fn token_claims(token: &str) -> Result<TokenClaims> {
    decode_json(
        token.split('.').nth(1).ok_or_else(|| {
            SandboxError::CloudProvider("Azure attestation evidence is not a JWT".into())
        })?,
        "claims",
    )
}

fn decode_json<T: serde::de::DeserializeOwned>(encoded: &str, part: &str) -> Result<T> {
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        SandboxError::CloudProvider(format!("Azure attestation {part} is not decodable"))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|_| SandboxError::CloudProvider(format!("Azure attestation {part} is not valid")))
}

fn verify_rs256(token: &str, jwk: &Jwk) -> Result<()> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(SandboxError::CloudProvider(
            "Azure attestation evidence is not a JWT".into(),
        ));
    };
    let signed = format!("{header}.{payload}");
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| {
        SandboxError::CloudProvider("Azure attestation signature is not decodable".into())
    })?;
    let n = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.n).map_err(|_| {
        SandboxError::CloudProvider("Azure signing key modulus is not decodable".into())
    })?);
    let e = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.e).map_err(|_| {
        SandboxError::CloudProvider("Azure signing key exponent is not decodable".into())
    })?);
    let public_key = RsaPublicKey::new(n, e).map_err(|_| {
        SandboxError::CloudProvider("Azure attestation signing key is not usable".into())
    })?;
    let key = VerifyingKey::<Sha256>::new(public_key);
    let signature = Signature::try_from(signature.as_slice()).map_err(|_| {
        SandboxError::CloudProvider("Azure attestation signature has the wrong length".into())
    })?;
    key.verify(signed.as_bytes(), &signature).map_err(|_| {
        SandboxError::CloudProvider(
            "Azure attestation signature was not made by Azure Attestation".into(),
        )
    })
}

async fn signing_key(origin: &str, kid: &str) -> Result<Jwk> {
    let url = format!("{origin}/certs");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|err| {
            SandboxError::CloudProvider(format!("Azure attestation key client: {err}"))
        })?;
    let jwks: Jwks = client
        .get(&url)
        .send()
        .await
        .map_err(|err| {
            SandboxError::CloudProvider(format!("Azure attestation key fetch failed: {err}"))
        })?
        .error_for_status()
        .map_err(|err| {
            SandboxError::CloudProvider(format!("Azure attestation key fetch failed: {err}"))
        })?
        .json()
        .await
        .map_err(|err| {
            SandboxError::CloudProvider(format!("Azure attestation keys were unreadable: {err}"))
        })?;
    jwks.keys
        .into_iter()
        .find(|key| {
            key.kid == kid
                && !key.n.is_empty()
                && !key.e.is_empty()
                && key.kty.as_deref().is_none_or(|kty| kty == "RSA")
                && key.alg.as_deref().is_none_or(|alg| alg == "RS256")
        })
        .ok_or_else(|| {
            SandboxError::CloudProvider("Azure attestation signing key was not published".into())
        })
}
