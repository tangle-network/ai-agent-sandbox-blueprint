//! Blueprint runner for ai-agent-sandbox-blueprint.

use ai_agent_sandbox_blueprint_lib::{JOB_WORKFLOW_TICK, bootstrap_workflows_from_chain, router};
use blueprint_producers_extra::cron::CronJob;
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::{TangleConsumer, TangleProducer};
use blueprint_sdk::{error, info};

#[cfg(feature = "qos")]
use blueprint_qos::QoSServiceBuilder;
#[cfg(feature = "qos")]
use blueprint_qos::heartbeat::{HeartbeatConfig, HeartbeatConsumer};
#[cfg(feature = "qos")]
use blueprint_qos::metrics::MetricsConfig;
#[cfg(feature = "qos")]
use std::sync::Arc;

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

                    // Spawn a background task that periodically pushes sandbox metrics
                    // from the lib's atomic counters to the QoS on-chain provider.
                    if let Some(provider) = qos_service.provider() {
                        let interval_secs = metrics_interval;
                        tokio::spawn(async move {
                            use blueprint_qos::metrics::types::MetricsProvider;

                            let mut interval = tokio::time::interval(
                                std::time::Duration::from_secs(interval_secs),
                            );
                            loop {
                                interval.tick().await;
                                let snapshot =
                                    ai_agent_sandbox_blueprint_lib::metrics::metrics().snapshot();
                                for (key, value) in snapshot {
                                    provider.add_on_chain_metric(key, value).await;
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    error!("Failed to initialize QoS service: {e} — continuing without QoS");
                }
            }
        }
    }

    // Spawn operator API server (provision progress, sandbox listing, session auth)
    {
        let api_port: u16 = std::env::var("OPERATOR_API_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9090);

        let router = sandbox_runtime::operator_api::operator_api_router();
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], api_port));
        info!("Starting operator API on {addr}");

        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            if let Err(e) = axum::serve(listener, router).await {
                error!("Operator API error: {e}");
            }
        });
    }

    // Load configuration from environment variables
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

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(reaper_interval));
            loop {
                interval.tick().await;
                ai_agent_sandbox_blueprint_lib::reaper::reaper_tick().await;
            }
        });

        // Spawn GC background task (stopped sandbox cleanup)
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(gc_interval));
            loop {
                interval.tick().await;
                ai_agent_sandbox_blueprint_lib::reaper::gc_tick().await;
            }
        });
    }

    // Create producer (listens for JobSubmitted events) and consumer (submits results)
    let tangle_producer = TangleProducer::new(tangle_client.clone(), service_id);
    let tangle_consumer = TangleConsumer::new(tangle_client);

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
    let result = BlueprintRunner::builder(tangle_config, env)
        .router(router())
        .producer(tangle_producer)
        .producer(workflow_cron)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async {
            info!("Shutting down ai-agent-sandbox-blueprint blueprint");
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
