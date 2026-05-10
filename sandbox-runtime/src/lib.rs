//! Shared sandbox container runtime for AI agent blueprints.
//!
//! This crate provides the core container lifecycle, storage, auth, metrics,
//! and garbage collection primitives that can be reused across multiple
//! blueprint implementations (event-driven, subscription, etc.).

pub mod api_types;
pub mod auth;
pub mod chat_state;
pub mod circuit_breaker;
pub mod contracts;
pub mod error;
pub mod firecracker;
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

/// Process-wide lock for tests that mutate env vars consumed by static
/// `OnceLock` / `Lazy` config (e.g. `SESSION_AUTH_SECRET`,
/// `FIRECRACKER_HOST_AGENT_*`, `TEE_BACKEND`, `BLUEPRINT_STATE_DIR`).
///
/// Without a single mutex shared across modules, each `#[test]` that
/// `set_var`s a config-relevant env interleaves with parallel tests. The
/// first init wins because `Lazy::new(...)` snapshots; subsequent tests
/// see stale config and either flake or assert against the wrong value.
///
/// Use it from any test module — both lib tests and integration tests in
/// `tests/` — by acquiring the lock before any `set_var` / `remove_var`:
///
/// ```ignore
/// let _guard = sandbox_runtime::TEST_ENV_GUARD
///     .lock()
///     .unwrap_or_else(|p| p.into_inner());
/// unsafe { std::env::set_var("…", "…") };
/// ```
///
/// Gated behind `cfg(any(test, feature = "test-utils"))` so the symbol
/// doesn't ship to production binaries.
#[cfg(any(test, feature = "test-utils"))]
pub static TEST_ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
