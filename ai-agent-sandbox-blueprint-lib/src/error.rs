use std::fmt;

/// Errors returned by sandbox blueprint operations.
#[derive(Debug)]
pub enum SandboxError {
    /// Authentication failure (invalid or missing token).
    Auth(String),
    /// Docker/container runtime failure.
    Docker(String),
    /// HTTP request to sidecar failed.
    Http(String),
    /// Invalid input or configuration.
    Validation(String),
    /// Requested resource not found.
    NotFound(String),
    /// Internal storage/state error.
    Storage(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::Auth(msg) => write!(f, "auth error: {msg}"),
            SandboxError::Docker(msg) => write!(f, "docker error: {msg}"),
            SandboxError::Http(msg) => write!(f, "http error: {msg}"),
            SandboxError::Validation(msg) => write!(f, "validation error: {msg}"),
            SandboxError::NotFound(msg) => write!(f, "not found: {msg}"),
            SandboxError::Storage(msg) => write!(f, "storage error: {msg}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// Convert SandboxError to String for blueprint job return types.
///
/// The blueprint SDK job handlers return `Result<T, String>`, so we need
/// this conversion. As the SDK evolves to support typed errors, this can
/// be replaced with a direct `impl IntoJobResult for SandboxError`.
impl From<SandboxError> for String {
    fn from(err: SandboxError) -> Self {
        err.to_string()
    }
}

pub type Result<T> = std::result::Result<T, SandboxError>;
