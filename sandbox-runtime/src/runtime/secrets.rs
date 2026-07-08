use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// At-rest encryption for secrets stored in SandboxRecord
// ─────────────────────────────────────────────────────────────────────────────

/// Prefix that marks a field as encrypted (enables transparent migration).
pub(crate) const ENC_PREFIX: &str = "enc:v1:";

/// HKDF info parameter for secrets-at-rest key derivation (distinct from PASETO).
pub(crate) const SECRETS_HKDF_INFO: &[u8] = b"secrets-at-rest-encryption-v1";

/// HKDF salt — shared with session_auth to derive from the same root secret,
/// but the distinct `info` parameter ensures an independent key.
pub(crate) const SECRETS_HKDF_SALT: &[u8] = b"tangle-sandbox-blueprint-paseto-v4";

/// 256-bit encryption key derived from `SESSION_AUTH_SECRET` via HKDF-SHA256.
/// Falls back to an ephemeral random key (with warning) if the env var is unset.
///
/// The key is wrapped in [`zeroize::Zeroizing`] so the underlying bytes are
/// wiped if the static is ever dropped, and so accidental clones carry the
/// same drop-wipe contract. The env-var `String` carrying the input keying
/// material is also explicitly zeroized after derivation.
pub(crate) static SEAL_KEY: once_cell::sync::Lazy<zeroize::Zeroizing<[u8; 32]>> =
    once_cell::sync::Lazy::new(|| {
        use hkdf::Hkdf;
        use sha2::Sha256;
        use zeroize::{Zeroize, Zeroizing};

        match std::env::var("SESSION_AUTH_SECRET") {
            Ok(mut secret) => {
                let hk = Hkdf::<Sha256>::new(Some(SECRETS_HKDF_SALT), secret.as_bytes());
                let mut key = Zeroizing::new([0u8; 32]);
                hk.expand(SECRETS_HKDF_INFO, &mut *key)
                    .expect("HKDF-SHA256 expand to 32 bytes cannot fail");
                secret.zeroize();
                key
            }
            Err(_) => {
                tracing::warn!(
                    "SESSION_AUTH_SECRET not set; using ephemeral key for secrets encryption. \
                 Stored secrets will NOT survive restart."
                );
                let mut key = Zeroizing::new([0u8; 32]);
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut *key);
                key
            }
        }
    });

/// Encrypt a plaintext string using ChaCha20-Poly1305 AEAD.
/// Returns `"enc:v1:" + base64(nonce || ciphertext)`.
pub(crate) fn seal_field(plaintext: &str) -> Result<String> {
    use base64::Engine;
    use chacha20poly1305::{
        AeadCore, ChaCha20Poly1305, KeyInit,
        aead::{Aead, OsRng},
    };

    if plaintext.is_empty() {
        return Ok(String::new());
    }

    let cipher = ChaCha20Poly1305::new((&**SEAL_KEY).into());
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| SandboxError::Storage(format!("seal_field encrypt failed: {e}")))?;

    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);

    Ok(format!(
        "{ENC_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    ))
}

/// Decrypt a stored field. If it doesn't carry the `enc:v1:` prefix, return as-is
/// (transparent migration from plaintext).
pub(crate) fn unseal_field(stored: &str) -> Result<String> {
    use base64::Engine;
    use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::Aead};

    if stored.is_empty() {
        return Ok(stored.to_string());
    }
    if !stored.starts_with(ENC_PREFIX) {
        // Migration path: pre-encryption records stored as plaintext.
        // This passthrough will be removed in a future release.
        tracing::warn!(
            "unseal_field: found unencrypted value — records will be re-encrypted on next write"
        );
        return Ok(stored.to_string());
    }

    let encoded = &stored[ENC_PREFIX.len()..];
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| SandboxError::Storage(format!("unseal_field base64 decode failed: {e}")))?;

    if blob.len() < 12 {
        return Err(SandboxError::Storage(
            "unseal_field: ciphertext too short".into(),
        ));
    }

    let nonce = chacha20poly1305::Nonce::from_slice(&blob[..12]);
    let ciphertext = &blob[12..];

    let cipher = ChaCha20Poly1305::new((&**SEAL_KEY).into());
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| SandboxError::Storage(format!("unseal_field decrypt failed: {e}")))?;

    String::from_utf8(plaintext)
        .map_err(|e| SandboxError::Storage(format!("unseal_field utf8 failed: {e}")))
}

/// Encrypt sensitive fields in a `SandboxRecord` before persisting.
///
/// Returns an error if any field fails to encrypt — never falls back to
/// storing plaintext, which would silently expose secrets at rest.
pub fn seal_record(record: &mut SandboxRecord) -> Result<()> {
    record.token =
        seal_field(&record.token).map_err(|e| SandboxError::Storage(format!("seal token: {e}")))?;
    record.base_env_json = seal_field(&record.base_env_json)
        .map_err(|e| SandboxError::Storage(format!("seal base_env_json: {e}")))?;
    record.user_env_json = seal_field(&record.user_env_json)
        .map_err(|e| SandboxError::Storage(format!("seal user_env_json: {e}")))?;
    Ok(())
}

/// Decrypt sensitive fields in a `SandboxRecord` after reading from store.
///
/// Returns an error if any field fails to decrypt. This prevents passing
/// garbled ciphertext to sidecars as credentials or environment variables.
pub fn unseal_record(record: &mut SandboxRecord) -> Result<()> {
    record.token = unseal_field(&record.token)
        .map_err(|e| SandboxError::Storage(format!("unseal token: {e}")))?;
    record.base_env_json = unseal_field(&record.base_env_json)
        .map_err(|e| SandboxError::Storage(format!("unseal base_env_json: {e}")))?;
    record.user_env_json = unseal_field(&record.user_env_json)
        .map_err(|e| SandboxError::Storage(format!("unseal user_env_json: {e}")))?;
    Ok(())
}
