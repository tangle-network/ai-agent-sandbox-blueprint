//! session_auth unit tests.

use super::*;

#[test]
fn challenge_lifecycle() {
    let _guard = capacity_test_lock();
    let challenge = create_challenge().unwrap();
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
    let _guard = capacity_test_lock();
    // Clear any leftover challenges from capacity tests to avoid
    // hitting the capacity cap when inserting our test challenge.
    CHALLENGES.lock().unwrap().clear();

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
    assert!(
        err_msg.contains("expired"),
        "Expected 'expired' in error: {err_msg}"
    );
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
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
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
    let _guard = capacity_test_lock();
    use k256::ecdsa::SigningKey;

    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    // Derive expected address
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let address_hash = keccak256(pubkey_uncompressed);
    let expected_address = format!("0x{}", hex::encode(&address_hash[12..]));

    // Step 1: Create challenge
    let challenge = create_challenge().unwrap();

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
    let _guard = capacity_test_lock();
    // Clear maps to avoid capacity interference from other tests
    CHALLENGES.lock().unwrap().clear();
    SESSIONS.lock().unwrap().clear();

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
    assert_eq!(extract_bearer_token("Bearer abc123"), Some("abc123"));
    assert_eq!(extract_bearer_token("bearer xyz"), Some("xyz"));
    assert_eq!(extract_bearer_token("BEARER token"), Some("token"));
    assert_eq!(extract_bearer_token("bEaReR Mixed"), Some("Mixed"));
    assert_eq!(extract_bearer_token("Bearer"), None);
    assert_eq!(extract_bearer_token("Bearer   "), None);
    assert_eq!(extract_bearer_token("Bearer a b"), None);
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
        *hkdf_key, keccak_hash,
        "HKDF output must differ from raw Keccak256"
    );
}

#[test]
fn challenge_capacity_blocks_when_full() {
    let _guard = capacity_test_lock();

    {
        let mut map = CHALLENGES.lock().unwrap();
        for i in 0..MAX_CHALLENGES {
            map.insert(
                format!("cap-ch-{i}"),
                Challenge {
                    nonce: format!("cap-ch-{i}"),
                    message: "cap".into(),
                    expires_at: now_secs() + 600,
                },
            );
        }
    }

    let result = create_challenge();

    // Clean up before assertions
    CHALLENGES
        .lock()
        .unwrap()
        .retain(|k, _| !k.starts_with("cap-ch-"));

    assert!(result.is_err(), "should fail when at capacity");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Challenge capacity exceeded"),
        "error should mention capacity: {err_msg}"
    );
}

#[test]
fn gc_restores_challenge_capacity_after_expiry() {
    let _guard = capacity_test_lock();

    {
        let mut map = CHALLENGES.lock().unwrap();
        for i in 0..MAX_CHALLENGES {
            map.insert(
                format!("gc-ch-{i}"),
                Challenge {
                    nonce: format!("gc-ch-{i}"),
                    message: "expired".into(),
                    expires_at: now_secs().saturating_sub(1),
                },
            );
        }
    }

    gc_sessions();

    let result = create_challenge();

    if let Ok(ref c) = result {
        CHALLENGES.lock().unwrap().remove(&c.nonce);
    }

    assert!(result.is_ok(), "should succeed after GC frees capacity");
}

#[test]
fn session_capacity_blocks_when_full() {
    let _guard = capacity_test_lock();

    {
        let mut map = SESSIONS.lock().unwrap();
        for i in 0..MAX_SESSIONS {
            map.insert(
                format!("cap-sess-{i}"),
                SessionClaims {
                    address: "0xdead".into(),
                    issued_at: now_secs(),
                    expires_at: now_secs() + 600,
                },
            );
        }
    }

    use k256::ecdsa::SigningKey;
    let signing_key = SigningKey::random(&mut OsRng);
    let challenge = create_challenge().unwrap();
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

    let result = exchange_signature_for_token(&challenge.nonce, &sig_hex);

    // Clean up before assertions
    SESSIONS
        .lock()
        .unwrap()
        .retain(|k, _| !k.starts_with("cap-sess-"));

    assert!(result.is_err(), "should fail when sessions are at capacity");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Session capacity exceeded"),
        "error should mention session capacity: {err_msg}"
    );
}

#[test]
fn create_test_token_produces_valid_session() {
    let addr = "0xabcdef1234567890abcdef1234567890abcdef12";
    let token = create_test_token(addr);
    assert!(
        token.starts_with("v4.local."),
        "test token should be a PASETO v4 local token"
    );

    let claims = validate_session_token(&token).unwrap();
    assert_eq!(claims.address, addr);
    assert!(claims.expires_at > now_secs());
}

// ── Adversarial: Token Revocation Effectiveness ────────────────────

#[test]
fn revoked_token_rejected_via_paseto_fallback() {
    let _guard = capacity_test_lock();
    clear_all_for_testing();

    let addr = "0x1111111111111111111111111111111111111111";
    let token = create_test_token(addr);

    // Token should validate before revocation
    assert!(
        validate_session_token(&token).is_ok(),
        "token must validate before revocation"
    );

    // Revoke the token
    let revoked = revoke_session(&token);
    assert!(
        revoked,
        "revoke_session should return true for active token"
    );

    // Token must NOT validate after revocation — even though the PASETO
    // is cryptographically valid, the revocation blacklist must block it.
    let result = validate_session_token(&token);
    assert!(
        result.is_err(),
        "CRITICAL: revoked token still validates! The PASETO fallback bypasses revocation."
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("revoked"),
        "error should mention revocation: {err_msg}"
    );
}

#[test]
fn revoke_sessions_for_address_blocks_paseto_fallback() {
    let _guard = capacity_test_lock();
    clear_all_for_testing();

    let addr = "0x2222222222222222222222222222222222222222";
    let token1 = create_test_token(addr);
    let token2 = create_test_token(addr);

    // Both tokens should validate
    assert!(validate_session_token(&token1).is_ok());
    assert!(validate_session_token(&token2).is_ok());

    // Revoke all sessions for this address
    let count = revoke_sessions_for_address(addr);
    assert_eq!(count, 2, "should revoke both sessions");

    // Neither token should validate anymore
    assert!(
        validate_session_token(&token1).is_err(),
        "token1 should be rejected after address-wide revocation"
    );
    assert!(
        validate_session_token(&token2).is_err(),
        "token2 should be rejected after address-wide revocation"
    );
}

#[test]
fn revocation_blacklist_gc_removes_expired_entries() {
    let _guard = capacity_test_lock();
    clear_all_for_testing();

    // Insert an already-expired entry directly into REVOKED
    REVOKED.lock().unwrap().insert(
        "expired-revoked-token".to_string(),
        now_secs().saturating_sub(1),
    );

    assert!(
        REVOKED
            .lock()
            .unwrap()
            .contains_key("expired-revoked-token")
    );

    gc_sessions();

    assert!(
        !REVOKED
            .lock()
            .unwrap()
            .contains_key("expired-revoked-token"),
        "GC should remove expired revocation blacklist entries"
    );
}

#[test]
fn revoke_unknown_token_still_blacklists() {
    let _guard = capacity_test_lock();
    clear_all_for_testing();

    // Revoke a token that isn't in the session store
    let revoked = revoke_session("v4.local.never-existed");
    assert!(!revoked, "should return false for unknown token");

    // But the token should still be in the blacklist
    assert!(
        REVOKED
            .lock()
            .unwrap()
            .contains_key("v4.local.never-existed"),
        "unknown token should be blacklisted defensively"
    );
}
