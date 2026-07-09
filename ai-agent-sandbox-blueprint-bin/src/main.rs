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

mod bootstrap;
mod consumer;
mod workflow_status;

use bootstrap::*;
use consumer::*;
use workflow_status::*;

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
    //
    // A single operator box can run one sandbox service per blueprint (e.g. a
    // legacy blueprint plus its redeployed successor). The BPM port allocator
    // honours the *preferred* port verbatim and fails ("Address already in use")
    // rather than falling back, so every sandbox service preferring the same
    // 9090 makes all but the first-reconciled service fail to bind. Offset the
    // preferred port by service_id (wrapping within the ephemeral range) so
    // co-located sandbox services request distinct ports. OPERATOR_API_PORT, when
    // set, pins an explicit base for deployments that manage ports externally.
    let base_port: u16 = std::env::var("OPERATOR_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);
    // Keep the offset small and bounded so it stays inside the manager's
    // allocatable range; service_id is unique per operator so this is collision-free
    // across co-located services on the same box.
    let preferred_port: u16 = base_port.wrapping_add((service_id % 1000) as u16);

    let (api_port, bind_addr) = if let Some(ref b) = bridge {
        let port = b
            .request_port(Some(preferred_port))
            .await
            .map_err(|e| blueprint_sdk::Error::Other(format!("BPM port allocation failed: {e}")))?;
        info!("BPM allocated port {port} for operator API (service {service_id}, preferred {preferred_port})");
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

    // A chain capacity above the host admission cap means the chain would
    // route work this host must reject — fail startup so the operator fixes
    // the configuration instead of serving capacity rejections.
    if let Err(msg) = validate_chain_vs_host_capacity(
        std::env::var("OPERATOR_MAX_CAPACITY").ok().as_deref(),
        std::env::var("SANDBOX_MAX_COUNT").ok().as_deref(),
    ) {
        return Err(blueprint_sdk::Error::Other(msg));
    }

    // Encode operator max capacity as registration inputs for the blueprint contract.
    // The contract's onRegister decodes abi.encode(uint32 capacity) from these inputs.
    let tangle_config = {
        let mut config = TangleConfig::default();
        if let Ok(cap_str) = std::env::var("OPERATOR_MAX_CAPACITY")
            && let Ok(capacity) = cap_str.parse::<u32>()
        {
            info!("Registering with OPERATOR_MAX_CAPACITY={capacity}");
            // ABI-encode a single uint32 (padded to 32 bytes)
            let mut inputs = vec![0u8; 32];
            inputs[28..32].copy_from_slice(&capacity.to_be_bytes());
            config = config.with_registration_inputs(inputs);
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

#[cfg(test)]
mod tests;
