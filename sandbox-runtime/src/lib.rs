//! Shared sandbox container runtime for AI agent blueprints.
//!
//! This crate provides the core container lifecycle, storage, auth, metrics,
//! and garbage collection primitives that can be reused across multiple
//! blueprint implementations (event-driven, subscription, etc.).

pub mod api_types;
pub mod auth;
pub mod circuit_breaker;
pub mod contracts;
pub mod error;
pub mod http;
pub mod ingress_access_control;
pub mod instance_types;
pub mod live_operator_sessions;
pub mod metrics;
pub mod operator_api;
pub mod provision_progress;
pub mod rate_limit;
pub mod reaper;
pub mod runtime;
pub mod scoped_session_auth;
pub mod secret_provisioning;
pub mod session_auth;
pub mod ssh_validation;
pub mod store;
pub mod tee;
pub mod util;

#[cfg(feature = "test-utils")]
pub mod test_utils;

pub use error::SandboxError;
pub use ingress_access_control::{
    AUTH_MODE_BEARER, DEFAULT_TOKEN_PREFIX, INGRESS_UI_AUTH_MODE_ENV, INGRESS_UI_BEARER_TOKEN_ENV,
    UiBearerCredential,
};
pub use runtime::{CreateSandboxParams, SandboxRecord, SandboxState};
pub use tee::{
    AttestationReport, TeeBackend, TeeConfig, TeeDeployParams, TeeDeployment, TeeType,
    init_tee_backend, tee_backend,
};

pub const DEFAULT_SIDECAR_IMAGE: &str = "ghcr.io/tangle-network/sidecar:latest";
pub const DEFAULT_SIDECAR_HTTP_PORT: u16 = 8080;
pub const DEFAULT_SIDECAR_SSH_PORT: u16 = 22;
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Maximum number of extra user-requested ports per sandbox.
pub const MAX_EXTRA_PORTS: usize = 8;
