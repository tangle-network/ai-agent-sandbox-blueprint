use crate::error::{Result, SandboxError};

pub fn normalize_username(username: &str) -> Result<String> {
    let trimmed = username.trim();
    let name = if trimmed.is_empty() { "root" } else { trimmed };
    crate::ssh_validation::validate_ssh_username(name).map_err(SandboxError::Validation)?;
    Ok(name.to_string())
}
