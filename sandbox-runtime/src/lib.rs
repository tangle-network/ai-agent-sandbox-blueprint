//! Shared sandbox container runtime for AI agent blueprints.
//!
//! This crate provides the core container lifecycle, storage, auth, metrics,
//! and garbage collection primitives that can be reused across multiple
//! blueprint implementations (event-driven, subscription, etc.).

pub mod api_types;
pub mod auth;
pub mod error;
pub mod http;
pub mod instance_types;
pub mod metrics;
pub mod operator_api;
pub mod provision_progress;
pub mod rate_limit;
pub mod reaper;
pub mod runtime;
pub mod secret_provisioning;
pub mod session_auth;
pub mod store;
pub mod tee;
pub mod util;

pub use error::SandboxError;
pub use runtime::{CreateSandboxParams, SandboxRecord, SandboxState};
pub use tee::{
    AttestationReport, TeeBackend, TeeConfig, TeeDeployParams, TeeDeployment, TeeType,
    init_tee_backend, tee_backend,
};

pub const DEFAULT_SIDECAR_IMAGE: &str = "ghcr.io/tangle-network/sidecar:latest";
pub const DEFAULT_SIDECAR_HTTP_PORT: u16 = 8080;
pub const DEFAULT_SIDECAR_SSH_PORT: u16 = 22;
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
