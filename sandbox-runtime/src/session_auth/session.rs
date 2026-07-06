//! PASETO v4.local session tokens: key derivation, issuance, validation,
//! revocation, and garbage collection of the in-memory stores.

use super::*;

/// Domain-specific salt for HKDF key derivation. This separates our key domain
/// from any other use of the same secret material.
const HKDF_SALT: &[u8] = b"tangle-sandbox-blueprint-paseto-v4";
/// HKDF info parameter for the PASETO symmetric key derivation.
const HKDF_INFO: &[u8] = b"session-auth-symmetric-key-v1";

/// Symmetric key for PASETO tokens. Derived once from `SESSION_AUTH_SECRET` env var
/// using HKDF-SHA256 (extract-then-expand), or a random key generated at startup.
///
/// **Warning**: When `SESSION_AUTH_SECRET` is not set, a random key is generated.
/// This means sessions will not survive a restart. Use [`validate_required_config`]
/// early in `main()` to enforce the secret is set in production.
static SYMMETRIC_KEY: Lazy<pasetors::keys::SymmetricKey<pasetors::version4::V4>> =
    Lazy::new(|| {
        let key_bytes: Zeroizing<[u8; 32]> = match std::env::var("SESSION_AUTH_SECRET") {
            Ok(mut secret) => {
                let derived = derive_symmetric_key(secret.as_bytes());
                secret.zeroize();
                derived
            }
            Err(_) => {
                tracing::error!(
                    "SESSION_AUTH_SECRET is not set — using random key. \
                 Sessions will NOT survive restart. Set this env var in production."
                );
                let mut bytes = Zeroizing::new([0u8; 32]);
                OsRng.fill_bytes(&mut *bytes);
                bytes
            }
        };
        pasetors::keys::SymmetricKey::<pasetors::version4::V4>::from(&*key_bytes)
            .expect("Failed to create PASETO symmetric key")
    });

/// Derive a 32-byte symmetric key from input keying material using HKDF-SHA256.
///
/// Returns the key in a [`Zeroizing`] wrapper so the temporary derivation
/// buffer is wiped from the heap when it goes out of scope — pasetors copies
/// the bytes into its own owned buffer when constructing `SymmetricKey`.
pub(crate) fn derive_symmetric_key(ikm: &[u8]) -> Zeroizing<[u8; 32]> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), ikm);
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(HKDF_INFO, &mut *key)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail");
    key
}

/// Verify a challenge signature and issue a PASETO session token.
pub fn exchange_signature_for_token(nonce: &str, signature_hex: &str) -> Result<SessionToken> {
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

    let token = pasetors::local::encrypt(&SYMMETRIC_KEY, &paseto_claims, None, None)
        .map_err(|e| SandboxError::Auth(format!("Failed to encrypt PASETO token: {e}")))?;

    // Store session for server-side validation (with capacity check)
    {
        let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        if sessions.len() >= MAX_SESSIONS {
            return Err(SandboxError::Unavailable(
                "Session capacity exceeded, try again later".into(),
            ));
        }
        sessions.insert(token.clone(), claims);
    }

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
        if let Some(claims) = sessions.get(token)
            && now_secs() <= claims.expires_at
        {
            return Ok(claims.clone());
        }
    }

    // Check revocation blacklist before PASETO fallback — a revoked token
    // must NOT authenticate even if cryptographically valid.
    {
        let revoked = REVOKED.lock().unwrap_or_else(|e| e.into_inner());
        if revoked.contains_key(token) {
            return Err(SandboxError::Auth("Session token has been revoked".into()));
        }
    }

    // Fall back to PASETO validation (for tokens surviving server restart)
    let validation = pasetors::token::UntrustedToken::try_from(token)
        .map_err(|e| SandboxError::Auth(format!("Invalid PASETO token: {e}")))?;

    let validation_rules = pasetors::claims::ClaimsValidationRules::new();
    let trusted =
        pasetors::local::decrypt(&SYMMETRIC_KEY, &validation, &validation_rules, None, None)
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
        .and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|dt| dt.unix_timestamp() as u64)
        .unwrap_or(0);

    // Parse expiration from PASETO standard "exp" field
    let exp_str = json
        .get("exp")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SandboxError::Auth("Missing expiration in token".into()))?;

    let exp_dt =
        time::OffsetDateTime::parse(exp_str, &time::format_description::well_known::Rfc3339)
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

/// Revoke a specific session token, removing it from the in-memory store
/// and adding it to the revocation blacklist so the PASETO fallback also
/// rejects it. The blacklist entry is kept until the token's original
/// expiration time, after which PASETO validation itself would reject it.
pub fn revoke_session(token: &str) -> bool {
    let claims = SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(token);

    if let Some(c) = &claims {
        REVOKED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.to_string(), c.expires_at);
    } else {
        // Token not in session store — still blacklist it with a 1-hour TTL
        // in case it's a valid PASETO token we don't have claims for.
        REVOKED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.to_string(), now_secs() + SESSION_TTL_SECS);
    }

    claims.is_some()
}

/// Revoke all sessions for a specific address.
/// Returns the number of sessions revoked.
pub fn revoke_sessions_for_address(address: &str) -> usize {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    let mut revoked = REVOKED.lock().unwrap_or_else(|e| e.into_inner());

    let mut count = 0usize;
    sessions.retain(|token, claims| {
        if claims.address.eq_ignore_ascii_case(address) {
            revoked.insert(token.clone(), claims.expires_at);
            count += 1;
            false
        } else {
            true
        }
    });
    count
}

/// Remove expired challenges, sessions, and revocation blacklist entries.
pub fn gc_sessions() {
    let now = now_secs();
    CHALLENGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, c| c.expires_at > now);
    SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, s| s.expires_at > now);
    REVOKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, expires_at| *expires_at > now);
}

/// Clear all challenges, sessions, and revocation blacklist entries.
/// Test/bench-only — prevents cross-test pollution when capacity tests fill
/// the global maps, and lets benches start from a clean slate.
#[cfg(any(test, feature = "test-utils"))]
pub fn clear_all_for_testing() {
    CHALLENGES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    REVOKED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Shared lock backing both sync and async capacity-test guards.
#[cfg(test)]
static CAPACITY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Acquire the shared capacity-test mutex (sync variant for `#[test]`).
/// Hold the returned guard for the duration of any test that creates
/// challenges or sessions to prevent races with capacity-exhaustion tests.
#[cfg(test)]
pub fn capacity_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    CAPACITY_LOCK.blocking_lock()
}

/// Async variant of [`capacity_test_lock`] for `#[tokio::test]` functions.
/// Can be held across `.await` points without triggering clippy warnings.
#[cfg(test)]
pub async fn capacity_test_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    CAPACITY_LOCK.lock().await
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

    let token = pasetors::local::encrypt(&SYMMETRIC_KEY, &paseto_claims, None, None).unwrap();
    SESSIONS.lock().unwrap().insert(token.clone(), claims);
    token
}
