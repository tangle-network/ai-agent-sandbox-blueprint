//! EIP-191 challenge-response + PASETO v4.local session tokens.
//!
//! Lightweight alternative to full `blueprint-auth` — avoids rocksdb, tonic,
//! and protobuf deps while providing multi-tenant wallet-based auth.
//!
//! Flow:
//! 1. Client requests a challenge: `POST /api/auth/challenge`
//! 2. Client signs the challenge with their wallet (EIP-191 personal_sign)
//! 3. Client exchanges the signature for a session token: `POST /api/auth/session`
//! 4. Client includes the PASETO token in `Authorization: Bearer <token>` headers

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use rand::RngCore;
use rand::rngs::OsRng;

use crate::error::{Result, SandboxError};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Challenge TTL in seconds (5 minutes).
const CHALLENGE_TTL_SECS: u64 = 300;
/// Session token TTL in seconds (1 hour).
const SESSION_TTL_SECS: u64 = 3600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Challenge {
    pub nonce: String,
    pub message: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionToken {
    pub token: String,
    pub address: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub address: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

// ---------------------------------------------------------------------------
// In-memory stores
// ---------------------------------------------------------------------------

static CHALLENGES: Lazy<Mutex<HashMap<String, Challenge>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static SESSIONS: Lazy<Mutex<HashMap<String, SessionClaims>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Challenge generation
// ---------------------------------------------------------------------------

/// Generate a random challenge nonce for EIP-191 signing.
pub fn create_challenge() -> Challenge {
    let mut nonce_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    let now = now_secs();

    let message = format!(
        "Sign this message to authenticate with Tangle Sandbox.\n\nNonce: {nonce}\nExpires: {}",
        now + CHALLENGE_TTL_SECS,
    );

    let challenge = Challenge {
        nonce: nonce.clone(),
        message,
        expires_at: now + CHALLENGE_TTL_SECS,
    };

    CHALLENGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(nonce, challenge.clone());

    challenge
}

/// Consume and validate a challenge nonce. Returns the challenge message if valid.
fn consume_challenge(nonce: &str) -> Result<String> {
    let mut map = CHALLENGES.lock().unwrap_or_else(|e| e.into_inner());
    let challenge = map
        .remove(nonce)
        .ok_or_else(|| SandboxError::Auth("Challenge not found or already consumed".into()))?;

    if now_secs() > challenge.expires_at {
        return Err(SandboxError::Auth("Challenge expired".into()));
    }

    Ok(challenge.message)
}

// ---------------------------------------------------------------------------
// EIP-191 signature verification via k256
// ---------------------------------------------------------------------------

/// Verify an EIP-191 personal_sign signature and return the recovered address.
///
/// The message is prefixed with `"\x19Ethereum Signed Message:\n{len}"` before
/// hashing with Keccak-256 and recovering the public key.
pub fn verify_eip191_signature(message: &str, signature_hex: &str) -> Result<String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let sig_bytes = hex::decode(signature_hex.trim_start_matches("0x"))
        .map_err(|e| SandboxError::Auth(format!("Invalid signature hex: {e}")))?;

    if sig_bytes.len() != 65 {
        return Err(SandboxError::Auth(format!(
            "Signature must be 65 bytes, got {}",
            sig_bytes.len()
        )));
    }

    // Split into r+s (64 bytes) and v (1 byte)
    let (rs, v_byte) = sig_bytes.split_at(64);
    let v = match v_byte[0] {
        0 | 27 => 0u8,
        1 | 28 => 1u8,
        v => {
            return Err(SandboxError::Auth(format!(
                "Invalid recovery id: {v}"
            )))
        }
    };

    let signature = Signature::from_slice(rs)
        .map_err(|e| SandboxError::Auth(format!("Invalid ECDSA signature: {e}")))?;

    let recovery_id = RecoveryId::new(v != 0, false);

    // EIP-191 prefix
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
    let digest = keccak256(prefixed.as_bytes());

    let verifying_key =
        VerifyingKey::recover_from_prehash(&digest, &signature, recovery_id)
            .map_err(|e| SandboxError::Auth(format!("Signature recovery failed: {e}")))?;

    // Derive address from uncompressed public key (skip 0x04 prefix byte)
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..]; // skip 0x04
    let address_hash = keccak256(pubkey_uncompressed);
    let address = format!("0x{}", hex::encode(&address_hash[12..]));

    Ok(address)
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

// ---------------------------------------------------------------------------
// Session token (PASETO v4.local)
// ---------------------------------------------------------------------------

/// Domain-specific salt for HKDF key derivation. This separates our key domain
/// from any other use of the same secret material.
const HKDF_SALT: &[u8] = b"tangle-sandbox-blueprint-paseto-v4";
/// HKDF info parameter for the PASETO symmetric key derivation.
const HKDF_INFO: &[u8] = b"session-auth-symmetric-key-v1";

/// Symmetric key for PASETO tokens. Derived once from `SESSION_AUTH_SECRET` env var
/// using HKDF-SHA256 (extract-then-expand), or a random key generated at startup.
static SYMMETRIC_KEY: Lazy<pasetors::keys::SymmetricKey<pasetors::version4::V4>> = Lazy::new(|| {
    let key_bytes = match std::env::var("SESSION_AUTH_SECRET") {
        Ok(secret) => {
            derive_symmetric_key(secret.as_bytes())
        }
        Err(_) => {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            bytes
        }
    };
    pasetors::keys::SymmetricKey::<pasetors::version4::V4>::from(&key_bytes)
        .expect("Failed to create PASETO symmetric key")
});

/// Derive a 32-byte symmetric key from input keying material using HKDF-SHA256.
///
/// Uses a domain-specific salt and info parameter to ensure the derived key is
/// unique to this application's PASETO token encryption, even if the same secret
/// is reused elsewhere.
fn derive_symmetric_key(ikm: &[u8]) -> [u8; 32] {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), ikm);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail");
    key
}

/// Verify a challenge signature and issue a PASETO session token.
pub fn exchange_signature_for_token(
    nonce: &str,
    signature_hex: &str,
) -> Result<SessionToken> {
    let message = consume_challenge(nonce)?;
    let address = verify_eip191_signature(&message, signature_hex)?;

    let now = now_secs();
    let expires_at = now + SESSION_TTL_SECS;

    let claims = SessionClaims {
        address: address.clone(),
        issued_at: now,
        expires_at,
    };

    // Build PASETO claims
    let mut paseto_claims = pasetors::claims::Claims::new()
        .map_err(|e| SandboxError::Auth(format!("Failed to create PASETO claims: {e}")))?;
    paseto_claims
        .add_additional("address", serde_json::json!(address))
        .map_err(|e| SandboxError::Auth(format!("Failed to add address claim: {e}")))?;
    // Set issued-at using the standard PASETO iat claim
    let iat_dt = time::OffsetDateTime::from_unix_timestamp(now as i64)
        .map_err(|e| SandboxError::Auth(format!("Invalid issued-at timestamp: {e}")))?;
    let iat_str = iat_dt
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| SandboxError::Auth(format!("Failed to format issued-at: {e}")))?;
    paseto_claims
        .issued_at(&iat_str)
        .map_err(|e| SandboxError::Auth(format!("Failed to set iat claim: {e}")))?;

    // Set expiration
    let exp_dt = time::OffsetDateTime::from_unix_timestamp(expires_at as i64)
        .map_err(|e| SandboxError::Auth(format!("Invalid expiration timestamp: {e}")))?;
    let exp_str = exp_dt
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| SandboxError::Auth(format!("Failed to format expiration: {e}")))?;
    paseto_claims
        .expiration(&exp_str)
        .map_err(|e| SandboxError::Auth(format!("Failed to set expiration: {e}")))?;

    let token = pasetors::local::encrypt(
        &*SYMMETRIC_KEY,
        &paseto_claims,
        None,
        None,
    )
    .map_err(|e| SandboxError::Auth(format!("Failed to encrypt PASETO token: {e}")))?;

    // Store session for server-side validation
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(token.clone(), claims);

    Ok(SessionToken {
        token,
        address,
        expires_at,
    })
}

/// Validate a PASETO session token and return the claims.
pub fn validate_session_token(token: &str) -> Result<SessionClaims> {
    // First try server-side session store (faster)
    {
        let sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(claims) = sessions.get(token) {
            if now_secs() <= claims.expires_at {
                return Ok(claims.clone());
            }
        }
    }

    // Fall back to PASETO validation (for tokens surviving server restart)
    let validation = pasetors::token::UntrustedToken::try_from(token)
        .map_err(|e| SandboxError::Auth(format!("Invalid PASETO token: {e}")))?;

    let validation_rules = pasetors::claims::ClaimsValidationRules::new();
    let trusted = pasetors::local::decrypt(
        &*SYMMETRIC_KEY,
        &validation,
        &validation_rules,
        None,
        None,
    )
    .map_err(|e| SandboxError::Auth(format!("PASETO decryption failed: {e}")))?;

    let payload = trusted.payload();
    let json: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| SandboxError::Auth(format!("Invalid token payload: {e}")))?;

    let address = json
        .get("address")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SandboxError::Auth("Missing address in token".into()))?
        .to_string();

    let iat = json
        .get("iat")
        .and_then(|v| v.as_str())
        .and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok())
        .map(|dt| dt.unix_timestamp() as u64)
        .unwrap_or(0);

    // Parse expiration from PASETO standard "exp" field
    let exp_str = json
        .get("exp")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SandboxError::Auth("Missing expiration in token".into()))?;

    let exp_dt = time::OffsetDateTime::parse(exp_str, &time::format_description::well_known::Rfc3339)
        .map_err(|e| SandboxError::Auth(format!("Invalid expiration format: {e}")))?;

    let expires_at = exp_dt.unix_timestamp() as u64;

    if now_secs() > expires_at {
        return Err(SandboxError::Auth("Session token expired".into()));
    }

    Ok(SessionClaims {
        address,
        issued_at: iat,
        expires_at,
    })
}

/// Remove expired challenges and sessions.
pub fn gc_sessions() {
    let now = now_secs();
    CHALLENGES.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, c| c.expires_at > now);
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, s| s.expires_at > now);
}

/// Extract a Bearer token from an Authorization header value.
pub fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header
        .strip_prefix("Bearer ")
        .or_else(|| auth_header.strip_prefix("bearer "))
        .map(|t| t.trim())
}

// ---------------------------------------------------------------------------
// Axum extractor — reusable across any blueprint's operator API
// ---------------------------------------------------------------------------

/// Axum extractor that validates the `Authorization: Bearer <token>` header
/// and yields the authenticated wallet address.
///
/// Usage in handler:
/// ```ignore
/// async fn my_handler(SessionAuth(address): SessionAuth) -> impl IntoResponse { ... }
/// ```
pub struct SessionAuth(pub String);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for SessionAuth {
    type Rejection = (axum::http::StatusCode, String);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    "Missing Authorization header".to_string(),
                )
            })?;

        let token = extract_bearer_token(auth_header).ok_or_else(|| {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format".to_string(),
            )
        })?;

        let claims = validate_session_token(token)
            .map_err(|e| (axum::http::StatusCode::UNAUTHORIZED, e.to_string()))?;

        Ok(SessionAuth(claims.address))
    }
}

/// Create a session token for a given address without going through EIP-191 signing.
/// Available in test builds and when the `test-utils` feature is enabled.
#[cfg(any(test, feature = "test-utils"))]
pub fn create_test_token(address: &str) -> String {
    let now = now_secs();
    let expires_at = now + SESSION_TTL_SECS;

    let claims = SessionClaims {
        address: address.to_string(),
        issued_at: now,
        expires_at,
    };

    let mut paseto_claims = pasetors::claims::Claims::new().unwrap();
    paseto_claims
        .add_additional("address", serde_json::json!(address))
        .unwrap();
    let iat_dt = time::OffsetDateTime::from_unix_timestamp(now as i64).unwrap();
    let iat_str = iat_dt
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    paseto_claims.issued_at(&iat_str).unwrap();
    let exp_dt = time::OffsetDateTime::from_unix_timestamp(expires_at as i64).unwrap();
    let exp_str = exp_dt
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    paseto_claims.expiration(&exp_str).unwrap();

    let token = pasetors::local::encrypt(&*SYMMETRIC_KEY, &paseto_claims, None, None).unwrap();
    SESSIONS.lock().unwrap().insert(token.clone(), claims);
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_lifecycle() {
        let challenge = create_challenge();
        assert!(!challenge.nonce.is_empty());
        assert!(challenge.message.contains(&challenge.nonce));
        assert!(challenge.expires_at > now_secs());

        // Should be consumable once
        let msg = consume_challenge(&challenge.nonce);
        assert!(msg.is_ok());

        // Should not be consumable again
        let msg2 = consume_challenge(&challenge.nonce);
        assert!(msg2.is_err());
    }

    #[test]
    fn challenge_expiry() {
        // Insert a challenge directly with an expired timestamp
        let nonce = "expired-test-nonce".to_string();
        let challenge = Challenge {
            nonce: nonce.clone(),
            message: "test message".into(),
            expires_at: now_secs().saturating_sub(10), // 10 seconds in the past
        };
        CHALLENGES.lock().unwrap().insert(nonce.clone(), challenge);

        let result = consume_challenge(&nonce);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("expired"), "Expected 'expired' in error: {err_msg}");
    }

    #[test]
    fn eip191_roundtrip() {
        use k256::ecdsa::SigningKey;

        // Generate a test signing key
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Derive the expected Ethereum address
        let pubkey_bytes = verifying_key.to_encoded_point(false);
        let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..]; // skip 0x04
        let address_hash = keccak256(pubkey_uncompressed);
        let expected_address = format!("0x{}", hex::encode(&address_hash[12..]));

        // Sign a message using EIP-191 personal_sign
        let message = "test message for signing";
        let prefixed = format!(
            "\x19Ethereum Signed Message:\n{}{}",
            message.len(),
            message
        );
        let digest = keccak256(prefixed.as_bytes());

        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&digest)
            .expect("signing failed");

        // Build the 65-byte signature (r || s || v)
        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&signature.to_bytes());
        sig_bytes.push(recovery_id.to_byte() + 27); // EIP-155 style v

        let sig_hex = format!("0x{}", hex::encode(&sig_bytes));

        // Verify
        let recovered = verify_eip191_signature(message, &sig_hex).unwrap();
        assert_eq!(recovered, expected_address);
    }

    #[test]
    fn token_roundtrip() {
        use k256::ecdsa::SigningKey;

        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Derive expected address
        let pubkey_bytes = verifying_key.to_encoded_point(false);
        let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
        let address_hash = keccak256(pubkey_uncompressed);
        let expected_address = format!("0x{}", hex::encode(&address_hash[12..]));

        // Step 1: Create challenge
        let challenge = create_challenge();

        // Step 2: Sign the challenge message
        let prefixed = format!(
            "\x19Ethereum Signed Message:\n{}{}",
            challenge.message.len(),
            challenge.message
        );
        let digest = keccak256(prefixed.as_bytes());

        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&digest)
            .expect("signing failed");

        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&signature.to_bytes());
        sig_bytes.push(recovery_id.to_byte() + 27);
        let sig_hex = format!("0x{}", hex::encode(&sig_bytes));

        // Step 3: Exchange for token
        let session_token = exchange_signature_for_token(&challenge.nonce, &sig_hex).unwrap();
        assert_eq!(session_token.address, expected_address);
        assert!(session_token.token.starts_with("v4.local."));
        assert!(session_token.expires_at > now_secs());

        // Step 4: Validate the token
        let claims = validate_session_token(&session_token.token).unwrap();
        assert_eq!(claims.address, expected_address);
        assert!(claims.expires_at > now_secs());
    }

    #[test]
    fn token_expiry_is_detected() {
        // Insert a session with an expired timestamp directly
        let token = "v4.local.fake-expired-token".to_string();
        let claims = SessionClaims {
            address: "0xdeadbeef".into(),
            issued_at: now_secs().saturating_sub(7200), // 2 hours ago
            expires_at: now_secs().saturating_sub(3600), // 1 hour ago (expired)
        };
        SESSIONS.lock().unwrap().insert(token.clone(), claims);

        // Server-side check should detect expiry
        let result = validate_session_token(&token);
        assert!(result.is_err());
    }

    #[test]
    fn gc_sessions_cleans_expired() {
        // Insert an expired challenge
        let expired_nonce = format!("gc-test-{}", now_secs());
        CHALLENGES.lock().unwrap().insert(
            expired_nonce.clone(),
            Challenge {
                nonce: expired_nonce.clone(),
                message: "expired".into(),
                expires_at: now_secs().saturating_sub(1),
            },
        );

        // Insert an expired session
        let expired_token = format!("gc-session-{}", now_secs());
        SESSIONS.lock().unwrap().insert(
            expired_token.clone(),
            SessionClaims {
                address: "0x1234".into(),
                issued_at: now_secs().saturating_sub(7200),
                expires_at: now_secs().saturating_sub(1),
            },
        );

        // Run GC
        gc_sessions();

        // Expired entries should be gone
        assert!(!CHALLENGES.lock().unwrap().contains_key(&expired_nonce));
        assert!(!SESSIONS.lock().unwrap().contains_key(&expired_token));
    }

    #[test]
    fn extract_bearer() {
        assert_eq!(
            extract_bearer_token("Bearer abc123"),
            Some("abc123")
        );
        assert_eq!(
            extract_bearer_token("bearer xyz"),
            Some("xyz")
        );
        assert_eq!(extract_bearer_token("Basic abc"), None);
    }

    #[test]
    fn keccak256_works() {
        let hash = keccak256(b"hello");
        // Known keccak256 of "hello"
        assert_eq!(
            hex::encode(hash),
            "1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8"
        );
    }

    #[test]
    fn hkdf_key_derivation_is_deterministic() {
        let key1 = derive_symmetric_key(b"test-secret-material");
        let key2 = derive_symmetric_key(b"test-secret-material");
        assert_eq!(key1, key2, "Same input must produce same key");

        // Different input produces different key
        let key3 = derive_symmetric_key(b"different-secret");
        assert_ne!(key1, key3, "Different input must produce different key");
    }

    #[test]
    fn hkdf_key_differs_from_raw_hash() {
        // Ensure HKDF output is NOT the same as a raw SHA-256 or Keccak hash
        let input = b"test-secret-for-comparison";
        let hkdf_key = derive_symmetric_key(input);
        let keccak_hash = keccak256(input);
        assert_ne!(
            hkdf_key, keccak_hash,
            "HKDF output must differ from raw Keccak256"
        );
    }
}
