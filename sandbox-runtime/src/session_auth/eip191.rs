//! EIP-191 personal_sign signature verification via k256.

use super::*;

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
        v => return Err(SandboxError::Auth(format!("Invalid recovery id: {v}"))),
    };

    let signature = Signature::from_slice(rs)
        .map_err(|e| SandboxError::Auth(format!("Invalid ECDSA signature: {e}")))?;

    let recovery_id = RecoveryId::new(v != 0, false);

    // EIP-191 prefix
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", message.len(), message);
    let digest = keccak256(prefixed.as_bytes());

    let verifying_key = VerifyingKey::recover_from_prehash(&digest, &signature, recovery_id)
        .map_err(|e| SandboxError::Auth(format!("Signature recovery failed: {e}")))?;

    // Derive address from uncompressed public key (skip 0x04 prefix byte)
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..]; // skip 0x04
    let address_hash = keccak256(pubkey_uncompressed);
    let address = format!("0x{}", hex::encode(&address_hash[12..]));

    Ok(address)
}

pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}
