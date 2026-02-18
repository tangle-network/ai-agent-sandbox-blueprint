//! TEE sealed secrets types.
//!
//! Types for the operator-mediated sealed secrets flow where:
//! 1. TEE derives a key pair bound to its attestation measurement
//! 2. Client encrypts secrets to the TEE's public key
//! 3. Encrypted blob transits through the operator (who cannot decrypt)
//! 4. Only the TEE can decrypt inside the enclave
//!
//! This module is intentionally isolated — it can be removed without affecting
//! the existing 2-phase plaintext secret provisioning flow.

use super::AttestationReport;

/// A TEE-bound public key with its attestation proof.
///
/// The client verifies the attestation before encrypting secrets,
/// ensuring the key truly belongs to a genuine TEE with the expected
/// code measurement.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TeePublicKey {
    /// Key agreement algorithm, e.g. "x25519-hkdf-sha256".
    pub algorithm: String,
    /// Raw public key bytes (hex-encoded for JSON transport).
    pub public_key_bytes: Vec<u8>,
    /// Attestation report proving this key was derived inside the TEE.
    pub attestation: AttestationReport,
}

/// An encrypted secret blob that only the TEE can decrypt.
///
/// Produced by the client after verifying the [`TeePublicKey`] attestation.
/// The operator forwards this opaque blob to the sidecar's sealed secrets
/// endpoint without being able to read the plaintext.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SealedSecret {
    /// Encryption algorithm used, e.g. "x25519-xsalsa20-poly1305".
    pub algorithm: String,
    /// Encrypted ciphertext.
    pub ciphertext: Vec<u8>,
    /// Nonce/IV used for encryption.
    pub nonce: Vec<u8>,
}

/// Response from the TEE after processing sealed secrets.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SealedSecretResult {
    /// Whether the secrets were successfully decrypted and applied.
    pub success: bool,
    /// Number of secret key-value pairs injected.
    pub secrets_count: usize,
    /// Error message if decryption or injection failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tee::TeeType;

    #[test]
    fn tee_public_key_roundtrip() {
        let pk = TeePublicKey {
            algorithm: "x25519-hkdf-sha256".to_string(),
            public_key_bytes: vec![1, 2, 3, 4],
            attestation: AttestationReport {
                tee_type: TeeType::Tdx,
                evidence: vec![10, 20],
                measurement: vec![30, 40],
                timestamp: 1234567890,
            },
        };
        let json = serde_json::to_string(&pk).unwrap();
        let decoded: TeePublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.algorithm, "x25519-hkdf-sha256");
        assert_eq!(decoded.public_key_bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn sealed_secret_roundtrip() {
        let ss = SealedSecret {
            algorithm: "x25519-xsalsa20-poly1305".to_string(),
            ciphertext: vec![0xDE, 0xAD],
            nonce: vec![0xBE, 0xEF],
        };
        let json = serde_json::to_string(&ss).unwrap();
        let decoded: SealedSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.algorithm, "x25519-xsalsa20-poly1305");
        assert_eq!(decoded.ciphertext, vec![0xDE, 0xAD]);
    }

    #[test]
    fn sealed_secret_result_roundtrip() {
        let r = SealedSecretResult {
            success: true,
            secrets_count: 3,
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("error")); // skip_serializing_if = None
        let decoded: SealedSecretResult = serde_json::from_str(&json).unwrap();
        assert!(decoded.success);
        assert_eq!(decoded.secrets_count, 3);
    }
}
