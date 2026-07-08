//! Axum-based operator API for sandbox management.
//!
//! Provides REST endpoints for:
//! - Listing active sandboxes
//! - Querying provision progress
//! - Session auth (challenge/response + PASETO tokens)
//! - Sandbox operations (exec, prompt, task, stop, resume, snapshot, SSH)

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::{
    Json, Router,
    body::Body,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio_stream::StreamExt;

use crate::api_types::*;
use crate::chat_state::{
    self, ChatMessageRecord, ChatRunKind, ChatRunProgressRecord, ChatRunRecord, ChatRunStatus,
    ChatSessionRecord,
};
use crate::circuit_breaker;
use crate::error::SandboxError;
use crate::http::{
    auth_headers, build_url, sidecar_get_json, sidecar_post_json, sidecar_post_json_without_timeout,
};
use crate::live_operator_sessions::sse_from_json_events;
use crate::metrics;
use crate::provision_progress;
use crate::rate_limit;
use crate::runtime::{
    self, SandboxRecord, SandboxState, sandboxes, workflow_runtime_credentials_available,
};
use crate::secret_provisioning;
use crate::session_auth::{self, SessionAuth};

// ---------------------------------------------------------------------------
// Per-operation sidecar call timeouts
// ---------------------------------------------------------------------------

/// Timeout for exec (shell command) calls to the sidecar.
const SIDECAR_EXEC_TIMEOUT: Duration = Duration::from_secs(30);

const DEFAULT_PROMPT_RUN_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const DEFAULT_TASK_RUN_TIMEOUT_MS: u64 = 30 * 60 * 1000;
const CHAT_CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for other sidecar calls (snapshot, SSH provisioning, etc.).
const SIDECAR_DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

const AGENT_WARMUP_ERROR_CODE: &str = "AGENT_WARMING_UP";
const TERMINAL_UNSUPPORTED_ERROR_CODE: &str = "TERMINAL_UNSUPPORTED";
const TERMINAL_PROMPT: &str = r"\u:\w\$ ";
#[cfg(not(test))]
const AGENT_WARMUP_RETRY_DELAYS_MS: &[u64] = &[250, 500, 1_000, 2_000, 4_000, 4_000, 4_000];
#[cfg(test)]
const AGENT_WARMUP_RETRY_DELAYS_MS: &[u64] = &[5, 5, 5];

static CHAT_RUN_ABORTS: Lazy<Mutex<HashMap<String, AbortHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static CHAT_RUN_ENQUEUE_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

mod admin;
mod agents;
mod auth;
mod chat;
mod chat_handlers;
mod chat_stream;
mod errors;
mod health;
mod lifecycle;
mod mw;
mod ports;
mod resolve;
mod sandboxes;
mod secrets;
mod sessions_core;
mod sessions_handlers;
mod sidecar_calls;
mod sidecar_core;
mod sse;
mod ssh;

pub(crate) use admin::*;
pub(crate) use agents::*;
pub(crate) use auth::*;
pub(crate) use chat::*;
pub(crate) use chat_handlers::*;
pub(crate) use chat_stream::*;
pub(crate) use errors::*;
pub(crate) use health::*;
pub(crate) use lifecycle::*;
pub(crate) use mw::*;
pub(crate) use ports::*;
pub(crate) use resolve::*;
pub(crate) use sandboxes::*;
pub(crate) use secrets::*;
pub(crate) use sessions_core::*;
pub(crate) use sessions_handlers::*;
pub(crate) use sidecar_calls::*;
pub(crate) use sidecar_core::*;
pub(crate) use sse::*;
pub(crate) use ssh::*;

// Externally-reachable items re-exported at their original (wider) visibility:
pub use errors::ApiError;
pub use mw::{RequestId, build_cors_layer, extract_session_from_headers};

// Router builder
// ---------------------------------------------------------------------------

/// Build the operator API router with all endpoints, CORS, and rate limiting.
///
/// For TEE-enabled operators, use [`operator_api_router_with_tee`] instead.
pub fn operator_api_router() -> Router {
    operator_api_router_with_tee_and_routes(None, Router::new())
}

/// Build the operator API router with optional TEE sealed secrets endpoints.
///
/// When `tee` is `Some(backend)`, the following endpoints are added:
/// - `GET  /api/sandboxes/{id}/tee/public-key`
/// - `POST /api/sandboxes/{id}/tee/sealed-secrets`
///
/// When `tee` is `None`, those routes are not registered and the router
/// behaves identically to [`operator_api_router`].
pub fn operator_api_router_with_tee(
    tee: Option<std::sync::Arc<dyn crate::tee::TeeBackend>>,
) -> Router {
    operator_api_router_with_tee_and_routes(tee, Router::new())
}

/// Build the operator API router and merge additional routes before applying
/// shared middleware such as CORS, request IDs, rate limits, and security
/// headers. This is important for blueprint-specific routes like
/// `/api/workflows/{workflow_id}` so browser preflight requests reach them too.
pub fn operator_api_router_with_tee_and_routes(
    tee: Option<std::sync::Arc<dyn crate::tee::TeeBackend>>,
    extra_routes: Router,
) -> Router {
    let cors = build_cors_layer();

    // Read endpoints: 120 req/min per IP
    let read_routes = Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .route(
            "/api/sandboxes/{sandbox_id}/ports",
            get(sandbox_ports_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/agents",
            get(sandbox_agents_handler),
        )
        .route("/api/sandbox/ports", get(instance_ports_handler))
        .route("/api/sandbox/agents", get(instance_agents_handler))
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions",
            get(sandbox_terminal_session_list_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}/stream",
            get(sandbox_terminal_session_stream_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions",
            get(sandbox_chat_session_list_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}",
            get(sandbox_chat_session_get_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}/stream",
            get(sandbox_chat_session_stream_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions",
            get(instance_terminal_session_list_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}/stream",
            get(instance_terminal_session_stream_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions",
            get(instance_chat_session_list_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}",
            get(instance_chat_session_get_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}/stream",
            get(instance_chat_session_stream_handler),
        )
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    // Write endpoints: 30 req/min per IP
    let write_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/secrets",
            get(get_secrets).post(inject_secrets).delete(wipe_secrets),
        )
        // Sidecar image upgrade (operator-gated; see handlers above).
        .route(
            "/api/operator/sidecar-image",
            get(sidecar_image_drift_handler),
        )
        .route(
            "/api/operator/sidecar-image/upgrade-stale",
            post(upgrade_stale_sidecar_images_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/upgrade-image",
            post(upgrade_sandbox_image_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions",
            post(sandbox_terminal_session_create_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}",
            axum::routing::delete(sandbox_terminal_session_delete_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions",
            post(sandbox_chat_session_create_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}",
            axum::routing::delete(sandbox_chat_session_delete_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/chat/sessions/{session_id}/runs/{run_id}/cancel",
            post(sandbox_chat_run_cancel_handler),
        )
        .route(
            "/api/sandbox/secrets",
            get(instance_get_secrets)
                .post(instance_inject_secrets)
                .delete(instance_wipe_secrets),
        )
        .route(
            "/api/sandbox/live/terminal/sessions",
            post(instance_terminal_session_create_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}",
            axum::routing::delete(instance_terminal_session_delete_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions",
            post(instance_chat_session_create_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}",
            axum::routing::delete(instance_chat_session_delete_handler),
        )
        .route(
            "/api/sandbox/live/chat/sessions/{session_id}/runs/{run_id}/cancel",
            post(instance_chat_run_cancel_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    let terminal_interactive_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}",
            patch(sandbox_terminal_session_resize_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/live/terminal/sessions/{session_id}/input",
            post(sandbox_terminal_session_input_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}",
            patch(instance_terminal_session_resize_handler),
        )
        .route(
            "/api/sandbox/live/terminal/sessions/{session_id}/input",
            post(instance_terminal_session_input_handler),
        )
        .layer(middleware::from_fn(
            rate_limit::terminal_interactive_rate_limit,
        ));

    // Sandbox-scoped operation endpoints (authenticated, write-rate-limited)
    let sandbox_op_routes = Router::new()
        .route(
            "/api/sandboxes/{sandbox_id}/exec",
            post(sandbox_exec_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/prompt",
            post(sandbox_prompt_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/task",
            post(sandbox_task_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/stop",
            post(sandbox_stop_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/resume",
            post(sandbox_resume_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/snapshot",
            post(sandbox_snapshot_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/ssh",
            post(sandbox_ssh_provision_handler).delete(sandbox_ssh_revoke_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/ssh/user",
            get(sandbox_ssh_user_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/port/{port}/{*rest}",
            any(sandbox_port_proxy_handler),
        )
        .route(
            "/api/sandboxes/{sandbox_id}/port/{port}",
            any(sandbox_port_proxy_root_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Instance-scoped operation endpoints (singleton sandbox, authenticated)
    let instance_op_routes = Router::new()
        .route("/api/sandbox/exec", post(instance_exec_handler))
        .route("/api/sandbox/prompt", post(instance_prompt_handler))
        .route("/api/sandbox/task", post(instance_task_handler))
        .route("/api/sandbox/stop", post(instance_stop_handler))
        .route("/api/sandbox/resume", post(instance_resume_handler))
        .route("/api/sandbox/snapshot", post(instance_snapshot_handler))
        .route(
            "/api/sandbox/ssh",
            post(instance_ssh_provision_handler).delete(instance_ssh_revoke_handler),
        )
        .route("/api/sandbox/ssh/user", get(instance_ssh_user_handler))
        .route(
            "/api/sandbox/port/{port}/{*rest}",
            any(instance_port_proxy_handler),
        )
        .route(
            "/api/sandbox/port/{port}",
            any(instance_port_proxy_root_handler),
        )
        .layer(middleware::from_fn(rate_limit::write_rate_limit));

    // Auth endpoints: 10 req/min per IP (stricter to prevent brute-force)
    let auth_routes = Router::new()
        .route("/api/auth/challenge", post(create_challenge))
        .route(
            "/api/auth/session",
            post(create_session).delete(revoke_session),
        )
        .layer(middleware::from_fn(rate_limit::auth_rate_limit));

    // Health, metrics & provision progress: rate-limited but unauthenticated
    // (liveness probes + pre-auth provision tracking need these)
    let infra_routes = Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/api/capabilities", get(capabilities_handler))
        .route("/metrics", get(prometheus_metrics))
        .route("/api/provisions", get(list_provisions))
        .route("/api/provisions/{call_id}", get(get_provision))
        .layer(middleware::from_fn(rate_limit::read_rate_limit));

    let mut router = Router::new()
        .merge(infra_routes)
        .merge(read_routes)
        .merge(write_routes)
        .merge(terminal_interactive_routes)
        .merge(sandbox_op_routes)
        .merge(instance_op_routes)
        .merge(auth_routes);

    // TEE sealed secrets endpoints (only when backend is configured)
    if let Some(backend) = tee {
        // The read-only attestation route is always available — it returns the
        // honest server-evaluated verdict and grants no trust by itself.
        let mut tee_routes = Router::new().route(
            "/api/sandboxes/{sandbox_id}/tee/attestation",
            get(crate::tee::sealed_secrets_api::get_tee_attestation)
                .post(crate::tee::sealed_secrets_api::post_tee_attestation),
        );

        // The trust-granting routes (public-key release, sealed-secret injection)
        // are mounted only when the server can fail closed: an allowlist is pinned
        // OR the operator explicitly opted into client-side-only verification.
        // With the default config and no allowlist they are not served at all, so
        // a misconfigured operator cannot hand back unverified material.
        if crate::tee::sealed_secrets_api::release_routes_enabled() {
            tee_routes = tee_routes
                .route(
                    "/api/sandboxes/{sandbox_id}/tee/public-key",
                    get(crate::tee::sealed_secrets_api::get_tee_public_key),
                )
                .route(
                    "/api/sandboxes/{sandbox_id}/tee/sealed-secrets",
                    post(crate::tee::sealed_secrets_api::inject_sealed_secrets),
                );
        } else {
            tracing::warn!(
                "TEE sealed-secret/public-key release routes disabled: no \
                 SANDBOX_TEE_EXPECTED_MEASUREMENTS allowlist is pinned. Set the allowlist, or set \
                 SANDBOX_TEE_REQUIRE_PINNED_MEASUREMENT=false to serve them under client-side-only \
                 verification."
            );
        }

        let tee_routes = tee_routes
            .layer(axum::Extension(
                Some(backend) as Option<std::sync::Arc<dyn crate::tee::TeeBackend>>
            ))
            .layer(middleware::from_fn(rate_limit::write_rate_limit));

        router = router.merge(tee_routes);
    }

    router
        .merge(extra_routes)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(http_metrics_middleware))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower::limit::ConcurrencyLimitLayer::new(64))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(120),
        ))
        .layer(cors)
        // Outermost layer: assign a unique request ID before anything else runs.
        .layer(middleware::from_fn(request_id_middleware))
}

#[cfg(test)]
mod tests;
