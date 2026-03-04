//! Shared SSH input validation primitives for operator/runtime APIs.

/// Maximum allowed SSH public key length.
pub(crate) const MAX_SSH_KEY_LEN: usize = 16 * 1024;

/// Maximum username length.
pub(crate) const MAX_USERNAME_LEN: usize = 32;

/// Accepted SSH key type prefixes.
const SSH_KEY_PREFIXES: &[&str] = &[
    "ssh-rsa ",
    "ssh-ed25519 ",
    "ssh-dss ",
    "ecdsa-sha2-nistp256 ",
    "ecdsa-sha2-nistp384 ",
    "ecdsa-sha2-nistp521 ",
    "sk-ssh-ed25519@openssh.com ",
    "sk-ecdsa-sha2-nistp256@openssh.com ",
];

/// Validate username (alphanumeric, dashes, underscores, dots; max 32 chars).
pub fn validate_ssh_username(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("ssh username must not be empty".to_string());
    }
    if trimmed.len() > MAX_USERNAME_LEN {
        return Err(format!(
            "ssh username exceeds maximum length ({MAX_USERNAME_LEN} chars)"
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("ssh username contains unsupported characters".to_string());
    }
    Ok(())
}

/// Validate SSH public key format and key type prefix.
pub fn validate_ssh_public_key(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("ssh public key must not be empty".to_string());
    }
    if trimmed.len() > MAX_SSH_KEY_LEN {
        return Err(format!(
            "ssh public key exceeds maximum length ({MAX_SSH_KEY_LEN} bytes)"
        ));
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err("ssh public key must be a single line".to_string());
    }
    if !SSH_KEY_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return Err("ssh public key must start with a supported key type".to_string());
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("ssh public key must contain key type and key data".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_ssh_public_key, validate_ssh_username};

    #[test]
    fn username_validation() {
        assert!(validate_ssh_username("agent_1").is_ok());
        assert!(validate_ssh_username("agent.name").is_ok());
        assert!(validate_ssh_username("").is_err());
        assert!(validate_ssh_username("bad name").is_err());
    }

    #[test]
    fn key_validation() {
        assert!(validate_ssh_public_key("ssh-ed25519 AAAA key").is_ok());
        assert!(validate_ssh_public_key("ssh-rsa AAAA key").is_ok());
        assert!(validate_ssh_public_key("").is_err());
        assert!(validate_ssh_public_key("invalid-key").is_err());
        assert!(validate_ssh_public_key("ssh-ed25519 AAAA\nnewline").is_err());
    }
}
