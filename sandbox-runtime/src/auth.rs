use rand::RngCore;
use rand::rngs::OsRng;

use crate::error::{Result, SandboxError};

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_from_request(override_token: &str) -> String {
    if override_token.trim().is_empty() {
        generate_token()
    } else {
        override_token.trim().to_string()
    }
}

pub fn require_sidecar_token(token: &str) -> Result<String> {
    if token.trim().is_empty() {
        return Err(SandboxError::Auth("sidecar_token is required".into()));
    }
    Ok(token.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_token_is_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_token_is_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }

    #[test]
    fn token_from_request_empty_generates() {
        let token = token_from_request("");
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn token_from_request_whitespace_generates() {
        let token = token_from_request("   ");
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn token_from_request_preserves_provided() {
        let token = token_from_request("  my-custom-token  ");
        assert_eq!(token, "my-custom-token");
    }

    #[test]
    fn require_sidecar_token_empty_fails() {
        let result = require_sidecar_token("");
        assert!(result.is_err());
    }

    #[test]
    fn require_sidecar_token_whitespace_fails() {
        let result = require_sidecar_token("   \t  ");
        assert!(result.is_err());
    }

    #[test]
    fn require_sidecar_token_valid() {
        let result = require_sidecar_token("  abc123  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123");
    }
}
