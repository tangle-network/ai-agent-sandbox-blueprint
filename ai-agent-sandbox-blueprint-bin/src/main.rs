//! Blueprint runner for ai-agent-sandbox-blueprint.

use ai_agent_sandbox_blueprint_lib::workflows::{
    WorkflowEntry, WorkflowStatusError, workflow_key, workflow_runtime_status_for_owner, workflows,
};
use ai_agent_sandbox_blueprint_lib::{
    JOB_WORKFLOW_TICK, JsonResponse, SandboxCreateOutput, bootstrap_workflows_from_chain, router,
};
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router as HttpRouter};
use blueprint_producers_extra::cron::CronJob;
use blueprint_sdk::alloy::sol_types::SolValue;
use blueprint_sdk::contexts::tangle::{TangleClient, TangleClientContext};
use blueprint_sdk::core::error::BoxError;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::TangleProducer;
use blueprint_sdk::tangle::extract::{CallId, ServiceId};
use blueprint_sdk::{error, info, warn};
use futures_util::Sink;
use serde_json::Value;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

#[cfg(feature = "qos")]
use blueprint_qos::QoSServiceBuilder;
#[cfg(feature = "qos")]
use blueprint_qos::heartbeat::{HeartbeatConfig, HeartbeatConsumer};
#[cfg(feature = "qos")]
use blueprint_qos::metrics::MetricsConfig;
#[cfg(feature = "qos")]
use std::sync::Arc;

fn workflow_status_error(error: WorkflowStatusError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &error {
        WorkflowStatusError::NotFound(_) => StatusCode::NOT_FOUND,
        WorkflowStatusError::Forbidden(_) => StatusCode::FORBIDDEN,
        WorkflowStatusError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (
        status,
        Json(serde_json::json!({
            "error": error.message(),
        })),
    )
}

async fn workflow_status_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
    Path(workflow_id): Path<u64>,
) -> Result<
    Json<ai_agent_sandbox_blueprint_lib::workflows::WorkflowRuntimeStatus>,
    (StatusCode, Json<serde_json::Value>),
> {
    workflow_runtime_status_for_owner(workflow_id, caller.as_str())
        .map(Json)
        .map_err(workflow_status_error)
}

async fn workflow_list_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
) -> Result<
    Json<serde_json::Value>,
    (StatusCode, Json<serde_json::Value>),
> {
    ai_agent_sandbox_blueprint_lib::workflows::list_workflows_for_owner(caller.as_str())
        .map(|workflows| {
            Json(serde_json::json!({
                "workflows": workflows
                    .into_iter()
                    .map(|workflow| serde_json::json!({
                        "scope": "sandbox",
                        "workflowId": workflow.workflow_id,
                        "name": workflow.name,
                        "triggerType": workflow.trigger_type,
                        "triggerConfig": workflow.trigger_config,
                        "targetKind": workflow.target_kind,
                        "targetSandboxId": workflow.target_sandbox_id,
                        "targetServiceId": workflow.target_service_id,
                        "active": workflow.active,
                        "running": workflow.running,
                        "lastRunAt": workflow.last_run_at,
                        "nextRunAt": workflow.next_run_at,
                        "latestExecution": workflow.latest_execution,
                    }))
                    .collect::<Vec<_>>(),
            }))
        })
        .map_err(workflow_status_error)
}

async fn workflow_detail_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
    Path(workflow_id): Path<u64>,
) -> Result<
    Json<serde_json::Value>,
    (StatusCode, Json<serde_json::Value>),
> {
    ai_agent_sandbox_blueprint_lib::workflows::workflow_detail_for_owner(
        workflow_id,
        caller.as_str(),
    )
    .map(|workflow| {
        Json(serde_json::json!({
            "scope": "sandbox",
            "workflowId": workflow.workflow_id,
            "name": workflow.name,
            "workflowJson": workflow.workflow_json,
            "triggerType": workflow.trigger_type,
            "triggerConfig": workflow.trigger_config,
            "sandboxConfigJson": workflow.sandbox_config_json,
            "targetKind": workflow.target_kind,
            "targetSandboxId": workflow.target_sandbox_id,
            "targetServiceId": workflow.target_service_id,
            "active": workflow.active,
            "running": workflow.running,
            "lastRunAt": workflow.last_run_at,
            "nextRunAt": workflow.next_run_at,
            "latestExecution": workflow.latest_execution,
        }))
    })
    .map_err(workflow_status_error)
}

fn workflow_status_router() -> HttpRouter {
    HttpRouter::new()
        .route("/api/workflows", get(workflow_list_handler))
        .route("/api/workflows/{workflow_id}", get(workflow_status_handler))
        .route("/api/workflows/{workflow_id}/detail", get(workflow_detail_handler))
}

/// Logging heartbeat consumer that records heartbeat submissions.
///
/// The actual on-chain submission is handled internally by `HeartbeatService`
/// via ECDSA signing + `submitHeartbeat` contract call. This consumer provides
/// a hook for blueprint-level logging/monitoring of heartbeat events.
#[cfg(feature = "qos")]
#[derive(Clone)]
struct LoggingHeartbeatConsumer;

#[cfg(feature = "qos")]
impl HeartbeatConsumer for LoggingHeartbeatConsumer {
    fn send_heartbeat(
        &self,
        status: &blueprint_qos::heartbeat::HeartbeatStatus,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = blueprint_qos::error::Result<()>> + Send + 'static>,
    > {
        let service_id = status.service_id;
        let status_code = status.status_code;
        let ts = status.timestamp;
        Box::pin(async move {
            info!("Heartbeat sent: service={service_id} status={status_code} ts={ts}");
            Ok(())
        })
    }
}

#[tokio::main]
#[allow(clippy::result_large_err)]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

    // Validate required auth config — SESSION_AUTH_SECRET must be set in production.
    // In test mode (--test-mode flag or TEST_MODE env var), log a warning but continue.
    let is_test_mode = std::env::args().any(|a| a == "--test-mode")
        || std::env::var("TEST_MODE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
    if let Err(msg) = sandbox_runtime::session_auth::validate_required_config() {
        if is_test_mode {
            warn!("Config validation (test mode): {msg}");
        } else {
            return Err(blueprint_sdk::Error::Other(msg));
        }
    }

    // QoS metrics provider is stored here for deferred spawn (after api_shutdown_tx exists).
    #[cfg(feature = "qos")]
    let mut qos_deferred: Option<(
        std::sync::Arc<blueprint_qos::metrics::provider::EnhancedMetricsProvider>,
        u64,
    )> = None;

    // Optionally start QoS background service (heartbeat + metrics collection + on-chain reporting)
    #[cfg(feature = "qos")]
    {
        let qos_enabled = std::env::var("QOS_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if qos_enabled {
            let metrics_interval = std::env::var("QOS_METRICS_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);

            let dry_run = std::env::var("QOS_DRY_RUN")
                .map(|v| v.eq_ignore_ascii_case("true"))
                .unwrap_or(true);

            // Build heartbeat config from environment
            let heartbeat_config = build_heartbeat_config();

            let mut builder = QoSServiceBuilder::<LoggingHeartbeatConsumer>::new()
                .with_metrics_config(MetricsConfig::default())
                .with_dry_run(dry_run);

            // Wire heartbeat if config is available (service_id and blueprint_id set)
            if let Some(hb_config) = heartbeat_config {
                let rpc_endpoint = std::env::var("HTTP_RPC_ENDPOINT")
                    .or_else(|_| std::env::var("RPC_URL"))
                    .unwrap_or_else(|_| "http://localhost:9944".to_string());

                let keystore_uri = std::env::var("KEYSTORE_URI")
                    .unwrap_or_else(|_| "file:///tmp/keystore".to_string());

                let registry_address = hb_config.status_registry_address;

                info!(
                    "Configuring heartbeat: service_id={}, blueprint_id={}, interval={}s, registry={}",
                    hb_config.service_id,
                    hb_config.blueprint_id,
                    hb_config.interval_secs,
                    registry_address,
                );

                builder = builder
                    .with_heartbeat_config(hb_config)
                    .with_heartbeat_consumer(Arc::new(LoggingHeartbeatConsumer))
                    .with_http_rpc_endpoint(rpc_endpoint)
                    .with_keystore_uri(keystore_uri)
                    .with_status_registry_address(registry_address);
            }

            match builder.build().await {
                Ok(qos_service) => {
                    info!(
                        "QoS service initialized (metrics_interval={metrics_interval}s, dry_run={dry_run})"
                    );

                    // Start heartbeat background task if configured
                    if let Some(hb) = qos_service.heartbeat_service() {
                        match hb.start_heartbeat().await {
                            Ok(()) => info!("Heartbeat service started"),
                            Err(e) => error!("Failed to start heartbeat: {e}"),
                        }
                    }

                    // Store QoS provider + interval for deferred spawn (after
                    // api_shutdown_tx is created — see below).
                    if let Some(provider) = qos_service.provider() {
                        qos_deferred = Some((provider, metrics_interval));
                    }
                }
                Err(e) => {
                    error!("Failed to initialize QoS service: {e} — continuing without QoS");
                }
            }
        }
    }

    // Optionally initialize TEE backend (when TEE_BACKEND env var is set)
    let tee_backend: Option<std::sync::Arc<dyn sandbox_runtime::tee::TeeBackend>> =
        if std::env::var("TEE_BACKEND").is_ok() {
            let backend = sandbox_runtime::tee::backend_factory::backend_from_env()
                .map_err(|e| blueprint_sdk::Error::Other(format!("TEE backend init: {e}")))?;
            let backend_type = format!("{:?}", backend.tee_type());
            ai_agent_sandbox_blueprint_lib::init_tee_backend(backend.clone());
            info!("TEE backend initialized (type: {backend_type})");
            Some(backend)
        } else {
            None
        };

    // Load configuration from environment variables (before API startup so we can
    // use the BPM bridge to determine binding address)
    let env = BlueprintEnvironment::load()?;

    // Connect to the Tangle network
    let tangle_client = env
        .tangle_client()
        .await
        .map_err(|e| blueprint_sdk::Error::Other(e.to_string()))?;

    // Get service ID from protocol settings
    let service_id = env
        .protocol_settings
        .tangle()
        .map_err(|e| blueprint_sdk::Error::Other(e.to_string()))?
        .service_id
        .ok_or_else(|| blueprint_sdk::Error::Other("SERVICE_ID missing".into()))?;

    info!("Starting ai-agent-sandbox-blueprint blueprint for service {service_id}");

    // Connect to the Blueprint Manager bridge. The BPM injects BRIDGE_SOCKET_PATH
    // when it spawns us. If the bridge is unavailable, behaviour depends on
    // ALLOW_STANDALONE: when true (dev only), bind 0.0.0.0 directly; when false
    // (the default for production), refuse to start.
    let allow_standalone = std::env::var("ALLOW_STANDALONE")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    let bridge = match env.bridge().await {
        Ok(b) => match b.ping().await {
            Ok(()) => {
                info!("Connected to Blueprint Manager bridge");
                Some(b)
            }
            Err(e) => {
                if allow_standalone {
                    warn!(
                        "Bridge ping failed ({e}), ALLOW_STANDALONE=true — running without proxy"
                    );
                    None
                } else {
                    return Err(blueprint_sdk::Error::Other(format!(
                        "BPM bridge ping failed: {e}. Set ALLOW_STANDALONE=true for dev mode."
                    )));
                }
            }
        },
        Err(e) => {
            if allow_standalone {
                warn!("No BPM bridge ({e}), ALLOW_STANDALONE=true — running without proxy");
                None
            } else {
                return Err(blueprint_sdk::Error::Other(format!(
                    "BPM bridge unavailable: {e}. Set ALLOW_STANDALONE=true for dev mode."
                )));
            }
        }
    };

    // Determine operator API port and binding address.
    // Behind BPM: request allocated port, bind 127.0.0.1 (only proxy can reach us).
    // Standalone: bind 0.0.0.0 on configured port (dev mode only).
    let preferred_port: u16 = std::env::var("OPERATOR_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);

    let (api_port, bind_addr) = if let Some(ref b) = bridge {
        let port = b
            .request_port(Some(preferred_port))
            .await
            .map_err(|e| blueprint_sdk::Error::Other(format!("BPM port allocation failed: {e}")))?;
        info!("BPM allocated port {port} for operator API");
        (port, [127, 0, 0, 1u8])
    } else {
        (preferred_port, [0, 0, 0, 0u8])
    };

    // Register with BPM proxy BEFORE starting the API server. This ensures the
    // proxy knows about us before any traffic can arrive, eliminating the race
    // window where the server is live but unregistered.
    if let Some(ref b) = bridge {
        let upstream_url = format!("http://127.0.0.1:{api_port}");
        let api_key_prefix = format!("svc{service_id}");

        b.register_blueprint_service_proxy(
            service_id,
            Some(api_key_prefix.as_str()),
            &upstream_url,
            &[],  // owners managed by BPM based on on-chain service registrants
            None, // TLS terminated by BPM proxy
        )
        .await
        .map_err(|e| {
            blueprint_sdk::Error::Other(format!(
                "BPM proxy registration failed: {e}. Cannot start without proxy."
            ))
        })?;

        info!(
            "Registered operator API with BPM proxy (service={service_id}, upstream={upstream_url})"
        );
    }

    // NOW start the API server — after registration is complete.
    let api_shutdown = tokio::sync::watch::channel(());
    let api_shutdown_tx = api_shutdown.0;
    let api_handle = {
        let router = sandbox_runtime::operator_api::operator_api_router_with_tee_and_routes(
            tee_backend,
            workflow_status_router(),
        );
        let addr = std::net::SocketAddr::from((bind_addr, api_port));
        info!("Starting operator API on {addr}");

        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            blueprint_sdk::Error::Other(format!("Failed to bind operator API on {addr}: {e}"))
        })?;

        let mut shutdown_rx = api_shutdown.1;
        tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await
            {
                error!("Operator API error: {e}");
            }
        })
    };

    if let Err(err) = bootstrap_workflows_from_chain(&tangle_client, service_id).await {
        error!("Failed to load workflows from chain: {err}");
    }

    // Reconcile stored sandbox state with Docker reality
    ai_agent_sandbox_blueprint_lib::reaper::reconcile_on_startup().await;

    // Spawn reaper background task (idle timeout + max lifetime enforcement)
    {
        let config = ai_agent_sandbox_blueprint_lib::runtime::SidecarRuntimeConfig::load();
        let reaper_interval = config.sandbox_reaper_interval;
        let gc_interval = config.sandbox_gc_interval;

        let mut reaper_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(reaper_interval));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Spawn each tick as a child task so panics are caught
                        // by JoinHandle instead of killing the loop.
                        let h = tokio::spawn(
                            ai_agent_sandbox_blueprint_lib::reaper::reaper_tick()
                        );
                        if let Err(e) = h.await {
                            error!("Reaper tick panicked: {e}");
                        }
                    }
                    _ = reaper_shutdown.changed() => {
                        info!("Reaper shutting down");
                        break;
                    }
                }
            }
        });

        // Spawn GC background task (stopped sandbox cleanup)
        let mut gc_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(gc_interval));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let h = tokio::spawn(
                            ai_agent_sandbox_blueprint_lib::reaper::gc_tick()
                        );
                        if let Err(e) = h.await {
                            error!("GC tick panicked: {e}");
                        }
                    }
                    _ = gc_shutdown.changed() => {
                        info!("GC shutting down");
                        break;
                    }
                }
            }
        });

        // Spawn session GC background task (expired challenges + sessions cleanup)
        let mut gc_session_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let h = tokio::spawn(async {
                            sandbox_runtime::session_auth::gc_sessions();
                        });
                        if let Err(e) = h.await {
                            error!("Session GC panicked: {e}");
                        }
                    }
                    _ = gc_session_shutdown.changed() => {
                        info!("Session GC shutting down");
                        break;
                    }
                }
            }
        });
    }

    // Spawn deferred QoS metrics loop now that api_shutdown_tx exists
    #[cfg(feature = "qos")]
    if let Some((provider, interval_secs)) = qos_deferred {
        let mut qos_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            use blueprint_qos::metrics::types::MetricsProvider;

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let snapshot =
                            ai_agent_sandbox_blueprint_lib::metrics::metrics().snapshot();
                        for (key, value) in snapshot {
                            provider.add_on_chain_metric(key, value).await;
                        }
                    }
                    _ = qos_shutdown.changed() => {
                        info!("QoS metrics loop shutting down");
                        break;
                    }
                }
            }
        });
    }

    // Create producer (listens for JobSubmitted events) and consumer (submits results)
    let tangle_producer = TangleProducer::new(tangle_client.clone(), service_id);
    let tangle_consumer = ReconciledTangleConsumer::new(tangle_client);

    // Encode operator max capacity as registration inputs for the blueprint contract.
    // The contract's onRegister decodes abi.encode(uint32 capacity) from these inputs.
    let tangle_config = {
        let mut config = TangleConfig::default();
        if let Ok(cap_str) = std::env::var("OPERATOR_MAX_CAPACITY") {
            if let Ok(capacity) = cap_str.parse::<u32>() {
                info!("Registering with OPERATOR_MAX_CAPACITY={capacity}");
                // ABI-encode a single uint32 (padded to 32 bytes)
                let mut inputs = vec![0u8; 32];
                inputs[28..32].copy_from_slice(&capacity.to_be_bytes());
                config = config.with_registration_inputs(inputs);
            }
        }
        config
    };
    let cron_schedule =
        std::env::var("WORKFLOW_CRON_SCHEDULE").unwrap_or_else(|_| "0 * * * * *".to_string());
    let workflow_cron = CronJob::new(JOB_WORKFLOW_TICK, cron_schedule.as_str())
        .await
        .map_err(|err| blueprint_sdk::Error::Other(format!("Invalid workflow cron: {err}")))?;

    // Build and run the blueprint
    let shutdown_bridge = bridge.clone();
    let result = BlueprintRunner::builder(tangle_config, env)
        .router(router())
        .producer(tangle_producer)
        .producer(workflow_cron)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async move {
            info!("Shutting down ai-agent-sandbox-blueprint blueprint");

            // Signal the API server to stop accepting new connections and drain in-flight requests.
            drop(api_shutdown_tx);
            match tokio::time::timeout(std::time::Duration::from_secs(10), api_handle).await {
                Ok(Ok(())) => info!("Operator API shut down cleanly"),
                Ok(Err(e)) => error!("Operator API task panicked: {e}"),
                Err(_) => warn!("Operator API shutdown timed out after 10s"),
            }

            // Only unregister from BPM AFTER the API is fully stopped, so the proxy
            // doesn't reject requests while we're still processing them.
            if let Some(b) = shutdown_bridge {
                if let Err(e) = b.unregister_blueprint_service_proxy(service_id).await {
                    error!("Failed to unregister from BPM proxy: {e}");
                } else {
                    info!("Unregistered from BPM proxy");
                }
            }
        })
        .run()
        .await;

    if let Err(e) = result {
        error!("Runner failed: {e:?}");
    }

    Ok(())
}

/// Build heartbeat config from environment variables.
///
/// Required env vars:
///   - `SERVICE_ID` or `TANGLE_SERVICE_ID` — the service instance ID
///   - `BLUEPRINT_ID` or `TANGLE_BLUEPRINT_ID` — the blueprint ID
///   - `STATUS_REGISTRY_ADDRESS` — the OperatorStatusRegistry contract address
///
/// Optional:
///   - `HEARTBEAT_INTERVAL_SECS` — heartbeat interval (default: 120)
///   - `HEARTBEAT_MAX_MISSED` — max missed beats before slashing (default: 3)
#[cfg(feature = "qos")]
fn build_heartbeat_config() -> Option<HeartbeatConfig> {
    use std::str::FromStr;

    let service_id: u64 = std::env::var("SERVICE_ID")
        .or_else(|_| std::env::var("TANGLE_SERVICE_ID"))
        .ok()
        .and_then(|v| v.parse().ok())?;

    let blueprint_id: u64 = std::env::var("BLUEPRINT_ID")
        .or_else(|_| std::env::var("TANGLE_BLUEPRINT_ID"))
        .ok()
        .and_then(|v| v.parse().ok())?;

    let registry_addr_str = std::env::var("STATUS_REGISTRY_ADDRESS").ok()?;
    let status_registry_address =
        blueprint_sdk::alloy::primitives::Address::from_str(&registry_addr_str).ok()?;

    let interval_secs: u64 = std::env::var("HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    let max_missed: u32 = std::env::var("HEARTBEAT_MAX_MISSED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    Some(HeartbeatConfig {
        interval_secs,
        jitter_percent: 10,
        service_id,
        blueprint_id,
        max_missed_heartbeats: max_missed,
        status_registry_address,
    })
}

fn setup_log() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{EnvFilter, fmt};
    if tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .is_err()
    {}
}

struct DerivedJobResult {
    service_id: u64,
    call_id: u64,
    output: blueprint_sdk::alloy::primitives::Bytes,
}

enum ConsumerState {
    WaitingForResult,
    ProcessingSubmission(
        Pin<Box<dyn std::future::Future<Output = Result<(), ReconciledConsumerError>> + Send>>,
    ),
}

impl ConsumerState {
    fn is_waiting(&self) -> bool {
        matches!(self, Self::WaitingForResult)
    }
}

#[derive(Debug, thiserror::Error)]
enum ReconciledConsumerError {
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(&'static str),
    #[error("Transaction error: {0}")]
    Transaction(String),
}

struct ReconciledTangleConsumer {
    client: Arc<TangleClient>,
    buffer: Mutex<VecDeque<DerivedJobResult>>,
    state: Mutex<ConsumerState>,
}

impl ReconciledTangleConsumer {
    fn new(client: TangleClient) -> Self {
        Self {
            client: Arc::new(client),
            buffer: Mutex::new(VecDeque::new()),
            state: Mutex::new(ConsumerState::WaitingForResult),
        }
    }
}

impl Sink<blueprint_sdk::JobResult> for ReconciledTangleConsumer {
    type Error = BoxError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: blueprint_sdk::JobResult) -> Result<(), Self::Error> {
        let blueprint_sdk::JobResult::Ok { head, body } = &item else {
            blueprint_sdk::trace!(target: "tangle-consumer", "Discarding job result with error");
            return Ok(());
        };

        let (Some(call_id_raw), Some(service_id_raw)) = (
            head.metadata.get(CallId::METADATA_KEY),
            head.metadata.get(ServiceId::METADATA_KEY),
        ) else {
            blueprint_sdk::trace!(
                target: "tangle-consumer",
                "Discarding job result with missing metadata"
            );
            return Ok(());
        };

        let call_id: u64 = call_id_raw
            .try_into()
            .map_err(|_| ReconciledConsumerError::InvalidMetadata("call_id"))?;
        let service_id: u64 = service_id_raw
            .try_into()
            .map_err(|_| ReconciledConsumerError::InvalidMetadata("service_id"))?;

        self.get_mut()
            .buffer
            .lock()
            .unwrap()
            .push_back(DerivedJobResult {
                service_id,
                call_id,
                output: blueprint_sdk::alloy::primitives::Bytes::copy_from_slice(body),
            });
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let consumer = self.get_mut();
        let mut state = consumer.state.lock().unwrap();

        {
            let buffer = consumer.buffer.lock().unwrap();
            if buffer.is_empty() && state.is_waiting() {
                return Poll::Ready(Ok(()));
            }
        }

        loop {
            match &mut *state {
                ConsumerState::WaitingForResult => {
                    let next = {
                        let mut buffer = consumer.buffer.lock().unwrap();
                        buffer.pop_front()
                    };

                    let Some(DerivedJobResult {
                        service_id,
                        call_id,
                        output,
                    }) = next
                    else {
                        return Poll::Ready(Ok(()));
                    };

                    let client = Arc::clone(&consumer.client);
                    let fut = Box::pin(async move {
                        submit_result_and_reconcile(client, service_id, call_id, output).await
                    });
                    *state = ConsumerState::ProcessingSubmission(fut);
                }
                ConsumerState::ProcessingSubmission(future) => match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(())) => {
                        *state = ConsumerState::WaitingForResult;
                    }
                    Poll::Ready(Err(err)) => {
                        *state = ConsumerState::WaitingForResult;
                        return Poll::Ready(Err(err.into()));
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let buffer = self.buffer.lock().unwrap();
        if buffer.is_empty() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

async fn submit_result_and_reconcile(
    client: Arc<TangleClient>,
    service_id: u64,
    call_id: u64,
    output: blueprint_sdk::alloy::primitives::Bytes,
) -> Result<(), ReconciledConsumerError> {
    if client.config.dry_run {
        info!(
            "Dry run enabled; skipping on-chain result submission for service {service_id} call {call_id}"
        );
        return Ok(());
    }

    match client
        .submit_result(service_id, call_id, output.clone())
        .await
    {
        Ok(result) if result.success => {
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Ok(result) => Err(ReconciledConsumerError::Transaction(format!(
            "Transaction reverted for service {service_id} call {call_id}: tx_hash={:?}",
            result.tx_hash
        ))),
        Err(err) if is_job_already_completed(&err.to_string()) => {
            warn!(
                "Result for service {service_id} call {call_id} was already completed; treating replay as idempotent"
            );
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Err(err)
            if replay_error_is_already_materialized(
                &client,
                service_id,
                call_id,
                &output,
                &err.to_string(),
            )
            .await =>
        {
            warn!(
                "Result for service {service_id} call {call_id} is already reflected on-chain; treating replay as idempotent"
            );
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Err(err) => Err(ReconciledConsumerError::Transaction(format!(
            "Failed to submit result for service {service_id} call {call_id}: {err}"
        ))),
    }
}

async fn reconcile_workflows(client: &TangleClient, service_id: u64) {
    if let Err(err) = bootstrap_workflows_from_chain(client, service_id).await {
        warn!("Failed to reconcile workflows from chain for service {service_id}: {err}");
    }
}

fn is_job_already_completed(error: &str) -> bool {
    error.contains("JobAlreadyCompleted") || error.contains("already completed")
}

async fn replay_error_is_already_materialized(
    client: &TangleClient,
    service_id: u64,
    call_id: u64,
    output: &blueprint_sdk::alloy::primitives::Bytes,
    error: &str,
) -> bool {
    if !error.contains("execution reverted") {
        return false;
    }

    if bootstrap_workflows_from_chain(client, service_id)
        .await
        .is_err()
    {
        return false;
    }

    let workflow_for_call_id = workflows()
        .ok()
        .and_then(|store| store.get(&workflow_key(call_id)).ok())
        .flatten();

    let payload = JsonResponse::abi_decode(output.as_ref())
        .ok()
        .and_then(|response| serde_json::from_str::<Value>(&response.json).ok());

    if let Some(payload) = payload.as_ref() {
        let Some(workflow_id) = payload.get("workflowId").and_then(Value::as_u64) else {
            return false;
        };

        let workflow = workflows()
            .ok()
            .and_then(|store| store.get(&workflow_key(workflow_id)).ok())
            .flatten();

        if workflow_replay_matches_store(call_id, payload, workflow.as_ref()) {
            return true;
        }
    }

    if let Ok(create_output) = SandboxCreateOutput::abi_decode(output.as_ref()) {
        if ai_agent_sandbox_blueprint_lib::runtime::get_sandbox_by_id(&create_output.sandboxId)
            .is_ok()
        {
            return true;
        }
    }

    // Workflow IDs are derived from the create call ID. If a replayed
    // `workflow_create` result arrives before we can decode its body cleanly,
    // an active workflow keyed by the same call ID is still enough evidence
    // that the original result has already been materialized on-chain.
    workflow_for_call_id
        .as_ref()
        .is_some_and(|entry| entry.active)
}

fn workflow_replay_matches_store(
    call_id: u64,
    payload: &Value,
    workflow: Option<&WorkflowEntry>,
) -> bool {
    let Some(workflow_id) = payload.get("workflowId").and_then(Value::as_u64) else {
        return false;
    };

    match payload.get("status").and_then(Value::as_str) {
        Some("canceled") => workflow.is_none(),
        Some("active") => workflow.as_ref().is_some(),
        _ if payload.get("task").is_some() => workflow.as_ref().is_some(),
        _ => workflow_id == call_id && workflow.as_ref().is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkflowEntry, workflow_replay_matches_store};
    use serde_json::json;

    fn active_workflow(id: u64) -> WorkflowEntry {
        WorkflowEntry {
            id,
            name: "workflow-qa".into(),
            workflow_json: "{}".into(),
            trigger_type: "cron".into(),
            trigger_config: "0 * * * * *".into(),
            sandbox_config_json: "{}".into(),
            target_kind: 0,
            target_sandbox_id: String::new(),
            target_service_id: 0,
            active: true,
            next_run_at: None,
            last_run_at: None,
            owner: String::new(),
        }
    }

    #[test]
    fn create_replay_matches_existing_active_workflow() {
        let payload = json!({
            "status": "active",
            "workflowId": 7
        });

        assert!(workflow_replay_matches_store(
            7,
            &payload,
            Some(&active_workflow(7))
        ));
    }

    #[test]
    fn trigger_replay_matches_existing_workflow_even_if_inactive_bit_isnt_rechecked() {
        let payload = json!({
            "status": "active",
            "workflowId": 9,
            "task": {
                "success": true
            }
        });

        assert!(workflow_replay_matches_store(
            12,
            &payload,
            Some(&active_workflow(9))
        ));
    }

    #[test]
    fn canceled_replay_only_matches_when_active_store_entry_is_absent() {
        let payload = json!({
            "status": "canceled",
            "workflowId": 11
        });

        assert!(workflow_replay_matches_store(15, &payload, None));
        assert!(!workflow_replay_matches_store(
            15,
            &payload,
            Some(&active_workflow(11))
        ));
    }
}
