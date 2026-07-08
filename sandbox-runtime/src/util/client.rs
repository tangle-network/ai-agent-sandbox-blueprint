use once_cell::sync::OnceCell;
use reqwest::Client;

use crate::error::{Result, SandboxError};

static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();
static HTTP_CLIENT_NO_TIMEOUT: OnceCell<Client> = OnceCell::new();

/// Get the shared HTTP client. The timeout is set from `SidecarRuntimeConfig`
/// on first initialization and reused for all subsequent calls.
pub fn http_client() -> Result<&'static Client> {
    HTTP_CLIENT
        .get_or_try_init(|| {
            let config = crate::runtime::SidecarRuntimeConfig::load();
            Client::builder()
                .timeout(config.timeout)
                .build()
                .map_err(|err| SandboxError::Http(format!("Failed to build HTTP client: {err}")))
        })
        .map_err(|err| SandboxError::Http(err.to_string()))
}

pub fn http_client_no_timeout() -> Result<&'static Client> {
    HTTP_CLIENT_NO_TIMEOUT
        .get_or_try_init(|| {
            Client::builder()
                .build()
                .map_err(|err| SandboxError::Http(format!("Failed to build HTTP client: {err}")))
        })
        .map_err(|err| SandboxError::Http(err.to_string()))
}
