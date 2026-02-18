//! Runtime backend selection via `TEE_BACKEND` environment variable.
//!
//! The TEE binary calls `backend_from_env()` at startup to construct the
//! appropriate `TeeBackend` implementation. Each backend reads its own
//! configuration from environment variables.
//!
//! # Supported values for `TEE_BACKEND`
//!
//! | Value              | Backend                     | Required env vars                        |
//! |--------------------|-----------------------------|------------------------------------------|
//! | `phala`            | Phala dstack (TDX)          | `PHALA_API_KEY`                          |
//! | `nitro` / `aws`    | AWS Nitro Enclaves          | `AWS_REGION`, `AWS_NITRO_*`              |
//! | `gcp`              | GCP Confidential Space      | `GCP_PROJECT_ID`, `GCP_ZONE`, etc.       |
//! | `azure`            | Azure Confidential VM + SKR | `AZURE_SUBSCRIPTION_ID`, etc.            |
//! | `direct`           | Operator-managed hardware   | `TEE_DIRECT_TYPE` (tdx/sev)              |

use std::sync::Arc;

use super::TeeBackend;
use crate::error::{Result, SandboxError};

/// Construct a `TeeBackend` based on the `TEE_BACKEND` environment variable.
///
/// Returns an `Arc<dyn TeeBackend>` ready to be passed to `init_tee_backend`.
pub fn backend_from_env() -> Result<Arc<dyn TeeBackend>> {
    let backend_name = std::env::var("TEE_BACKEND").map_err(|_| {
        SandboxError::Validation(
            "TEE_BACKEND environment variable is required. \
             Supported values: phala, nitro, aws, gcp, azure, direct"
                .to_string(),
        )
    })?;

    match backend_name.to_lowercase().as_str() {
        #[cfg(feature = "tee-phala")]
        "phala" => {
            let api_key = require_env("PHALA_API_KEY")?;
            let api_endpoint = std::env::var("PHALA_API_ENDPOINT").ok();
            let backend = super::phala::PhalaBackend::new(&api_key, api_endpoint)?;
            Ok(Arc::new(backend))
        }

        #[cfg(not(feature = "tee-phala"))]
        "phala" => Err(SandboxError::Validation(
            "Phala backend requested but the 'tee-phala' feature is not enabled. \
             Rebuild with --features tee-phala"
                .to_string(),
        )),

        #[cfg(feature = "tee-aws-nitro")]
        "nitro" | "aws" => {
            let config = super::aws_nitro::NitroConfig::from_env()?;
            Ok(Arc::new(super::aws_nitro::NitroBackend::new(config)))
        }

        #[cfg(not(feature = "tee-aws-nitro"))]
        "nitro" | "aws" => Err(SandboxError::Validation(
            "AWS Nitro backend requested but the 'tee-aws-nitro' feature is not enabled. \
             Rebuild with --features tee-aws-nitro"
                .to_string(),
        )),

        #[cfg(feature = "tee-gcp")]
        "gcp" => {
            let config = super::gcp::GcpConfig::from_env()?;
            Ok(Arc::new(super::gcp::GcpConfidentialSpaceBackend::new(
                config,
            )))
        }

        #[cfg(not(feature = "tee-gcp"))]
        "gcp" => Err(SandboxError::Validation(
            "GCP Confidential Space backend requested but the 'tee-gcp' feature is not enabled. \
             Rebuild with --features tee-gcp"
                .to_string(),
        )),

        #[cfg(feature = "tee-azure")]
        "azure" => {
            let config = super::azure::AzureConfig::from_env()?;
            Ok(Arc::new(super::azure::AzureSkrBackend::new(config)))
        }

        #[cfg(not(feature = "tee-azure"))]
        "azure" => Err(SandboxError::Validation(
            "Azure SKR backend requested but the 'tee-azure' feature is not enabled. \
             Rebuild with --features tee-azure"
                .to_string(),
        )),

        #[cfg(feature = "tee-direct")]
        "direct" => {
            let tee_type = match std::env::var("TEE_DIRECT_TYPE")
                .unwrap_or_default()
                .to_lowercase()
                .as_str()
            {
                "tdx" => super::TeeType::Tdx,
                "sev" => super::TeeType::Sev,
                "nitro" => super::TeeType::Nitro,
                other => {
                    return Err(SandboxError::Validation(format!(
                        "Invalid TEE_DIRECT_TYPE '{other}'. Supported: tdx, sev, nitro"
                    )));
                }
            };
            Ok(Arc::new(super::direct::DirectTeeBackend::new(tee_type)))
        }

        #[cfg(not(feature = "tee-direct"))]
        "direct" => Err(SandboxError::Validation(
            "Direct backend requested but the 'tee-direct' feature is not enabled. \
             Rebuild with --features tee-direct"
                .to_string(),
        )),

        other => Err(SandboxError::Validation(format!(
            "Unknown TEE_BACKEND '{other}'. Supported values: phala, nitro, aws, gcp, azure, direct"
        ))),
    }
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| {
        SandboxError::Validation(format!("{name} environment variable is required"))
    })
}
