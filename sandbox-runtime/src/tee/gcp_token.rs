//! Verify a Google Confidential Space attestation token.
//!
//! The launcher token is an RS256 JWT. Google publishes the signing keys at
//! the Confidential Space JWKS. This check runs in the operator before a TEE
//! create is allowed to succeed. It is not an on-chain check and it does not
//! prove a malicious operator ran this binary.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::sha2::Sha256;
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use subtle::ConstantTimeEq;

use crate::error::{Result, SandboxError};
use crate::tee::AttestationReport;

const ISSUER: &str = "https://confidentialcomputing.googleapis.com";
const JWKS_URL: &str = "https://www.googleapis.com/service_accounts/v1/metadata/jwk/signer@confidentialspace-sign.iam.gserviceaccount.com";
const PRODUCTION_DEBUG_STATUS: &str = "disabled-since-boot";
/// A Confidential Space token is a few kilobytes. A larger body is not one.
const MAX_EVIDENCE_BYTES: usize = 8192;

#[derive(serde::Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(serde::Deserialize)]
struct Jwk {
    kid: String,
    n: String,
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
    /// Critical extensions. An extension we do not implement must fail closed.
    #[serde(default)]
    crit: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct TokenClaims {
    iss: String,
    #[serde(default)]
    aud: Audience,
    exp: u64,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    sub: String,
    dbgstat: String,
    #[serde(default)]
    secboot: bool,
    #[serde(default)]
    swname: String,
    #[serde(default)]
    hwmodel: String,
    /// Echoed from the guest token request. Absent on tokens the guest did
    /// not challenge. A string or a list of strings.
    #[serde(default)]
    eat_nonce: EatNonce,
}

#[derive(Default, serde::Deserialize)]
#[serde(untagged)]
enum EatNonce {
    #[default]
    Missing,
    One(String),
    Many(Vec<String>),
}

impl EatNonce {
    fn values(&self) -> Vec<&str> {
        match self {
            Self::Missing => Vec::new(),
            Self::One(value) => vec![value.as_str()],
            Self::Many(values) => values.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(untagged)]
enum Audience {
    #[default]
    Missing,
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn present(&self) -> bool {
        match self {
            Self::Missing => false,
            Self::One(value) => !value.is_empty(),
            Self::Many(values) => values.iter().any(|value| !value.is_empty()),
        }
    }
}

/// Refuse a TEE create unless the sidecar returned a production Confidential
/// Space token signed by Google. When the caller supplied a nonce, the token
/// must echo it in `eat_nonce`. A token with no caller nonce is not required
/// to carry `eat_nonce`.
pub async fn require_production_token(
    report: &AttestationReport,
    expected_nonce: Option<&[u8; 64]>,
) -> Result<()> {
    if report.evidence.len() > MAX_EVIDENCE_BYTES {
        return Err(SandboxError::CloudProvider(
            "GCP attestation evidence is too large".into(),
        ));
    }
    let raw = std::str::from_utf8(&report.evidence).map_err(|_| {
        SandboxError::CloudProvider("GCP attestation evidence is not a token".into())
    })?;
    let owned = compact_jwt(raw)?;
    let token = owned.as_str();
    let header = token_header(token)?;
    if header.alg != "RS256" {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token must use RS256".into(),
        ));
    }
    if header.crit.as_ref().is_some_and(|crit| !crit.is_empty()) {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token has an unsupported critical header".into(),
        ));
    }

    let jwk = signing_key(&header.kid).await?;
    verify_rs256(token, &jwk)?;
    let claims = token_claims(token)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SandboxError::CloudProvider("system clock is before the unix epoch".into()))?
        .as_secs();
    if claims.exp.saturating_add(60) < now {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token is expired".into(),
        ));
    }
    if claims.nbf.is_some_and(|nbf| nbf > now + 60) {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token is not valid yet".into(),
        ));
    }
    if claims.iss != ISSUER {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token issuer was not Google Confidential Space".into(),
        ));
    }
    if !claims.aud.present() {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token has no audience".into(),
        ));
    }
    if claims.sub.is_empty() {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token has no subject".into(),
        ));
    }
    if claims.dbgstat != PRODUCTION_DEBUG_STATUS {
        return Err(SandboxError::CloudProvider(
            "GCP attestation is from a debug image, not a production Confidential Space image"
                .into(),
        ));
    }
    if !claims.secboot || claims.swname != "CONFIDENTIAL_SPACE" {
        return Err(SandboxError::CloudProvider(
            "GCP attestation is not a secure-boot Confidential Space token".into(),
        ));
    }
    if claims.hwmodel != "GCP_AMD_SEV" && claims.hwmodel != "GCP_AMD_SEV_SNP" {
        return Err(SandboxError::CloudProvider(
            "GCP attestation hardware model is not AMD SEV".into(),
        ));
    }
    if let Some(expected) = expected_nonce
        && !eat_nonce_binds(&claims.eat_nonce, expected)
    {
        return Err(SandboxError::CloudProvider(
            "GCP attestation token does not bind the caller nonce".into(),
        ));
    }
    Ok(())
}

fn eat_nonce_binds(claim: &EatNonce, expected: &[u8; 64]) -> bool {
    claim
        .values()
        .into_iter()
        .any(|value| nonce_text_matches(value, expected))
}

/// Accept the 64-byte nonce, or a shorter prefix whose right-pad is that nonce.
fn nonce_text_matches(value: &str, expected: &[u8; 64]) -> bool {
    let text = value.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if text.len() % 2 != 0 || text.len() > 128 || !text.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    let Ok(bytes) = hex::decode(text) else {
        return false;
    };
    if bytes.len() > 64 {
        return false;
    }
    let mut padded = [0u8; 64];
    padded[..bytes.len()].copy_from_slice(&bytes);
    padded.ct_eq(expected).into()
}

fn compact_jwt(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let body = unchunk(trimmed).unwrap_or(trimmed);
    let compact: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if compact.split('.').count() != 3 {
        return Err(SandboxError::CloudProvider(
            "GCP attestation evidence is not a JWT".into(),
        ));
    }
    Ok(compact)
}

/// The launcher socket speaks chunked HTTP. A sidecar that forwards the body
/// without decoding it leaves the hex chunk size in front of the token.
fn unchunk(raw: &str) -> Option<&str> {
    let (size, rest) = raw.split_once("\r\n")?;
    if size.is_empty() || !size.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let len = usize::from_str_radix(size, 16).ok()?;
    rest.get(..len)
}

fn token_header(token: &str) -> Result<Header> {
    let encoded = token.split('.').next().ok_or_else(|| {
        SandboxError::CloudProvider("GCP attestation evidence is not a JWT".into())
    })?;
    decode_json(encoded, "header")
}

fn token_claims(token: &str) -> Result<TokenClaims> {
    let encoded = token.split('.').nth(1).ok_or_else(|| {
        SandboxError::CloudProvider("GCP attestation evidence is not a JWT".into())
    })?;
    decode_json(encoded, "claims")
}

fn decode_json<T: serde::de::DeserializeOwned>(encoded: &str, part: &str) -> Result<T> {
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        SandboxError::CloudProvider(format!("GCP attestation {part} is not decodable"))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|_| SandboxError::CloudProvider(format!("GCP attestation {part} is not valid")))
}

fn verify_rs256(token: &str, jwk: &Jwk) -> Result<()> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(SandboxError::CloudProvider(
            "GCP attestation evidence is not a JWT".into(),
        ));
    };
    let signed = format!("{header}.{payload}");
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| {
        SandboxError::CloudProvider("GCP attestation signature is not decodable".into())
    })?;
    let n = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.n).map_err(|_| {
        SandboxError::CloudProvider("GCP signing key modulus is not decodable".into())
    })?);
    let e = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.e).map_err(|_| {
        SandboxError::CloudProvider("GCP signing key exponent is not decodable".into())
    })?);
    let public_key = RsaPublicKey::new(n, e).map_err(|_| {
        SandboxError::CloudProvider("GCP attestation signing key is not usable".into())
    })?;
    let key = VerifyingKey::<Sha256>::new(public_key);
    let signature = Signature::try_from(signature.as_slice()).map_err(|_| {
        SandboxError::CloudProvider("GCP attestation signature has the wrong length".into())
    })?;
    key.verify(signed.as_bytes(), &signature).map_err(|_| {
        SandboxError::CloudProvider("GCP attestation signature was not made by Google".into())
    })
}

async fn signing_key(kid: &str) -> Result<Jwk> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|err| SandboxError::CloudProvider(format!("JWKS client: {err}")))?;
    let jwks: Jwks = client
        .get(JWKS_URL)
        .send()
        .await
        .map_err(|err| {
            SandboxError::CloudProvider(format!("GCP attestation key fetch failed: {err}"))
        })?
        .error_for_status()
        .map_err(|err| {
            SandboxError::CloudProvider(format!("GCP attestation key fetch failed: {err}"))
        })?
        .json()
        .await
        .map_err(|err| {
            SandboxError::CloudProvider(format!("GCP attestation keys were unreadable: {err}"))
        })?;
    jwks.keys
        .into_iter()
        .find(|key| {
            key.kid == kid
                && key.kty.as_deref().is_none_or(|kty| kty == "RSA")
                && key.alg.as_deref().is_none_or(|alg| alg == "RS256")
        })
        .ok_or_else(|| {
            SandboxError::CloudProvider("GCP attestation signing key was not published".into())
        })
}
