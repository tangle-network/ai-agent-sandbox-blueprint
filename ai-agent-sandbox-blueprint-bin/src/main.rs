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
use blueprint_qos::metrics::MetricsConfig;
#[cfg(feature = "qos")]
use blueprint_qos::QoSServiceBuilder;
#[cfg(feature = "qos")]
use blueprint_qos::heartbeat::HeartbeatConsumer;

/// No-op heartbeat consumer for metrics-only QoS mode.
#[cfg(feature = "qos")]
#[derive(Clone)]
struct NoopHeartbeatConsumer;

#[cfg(feature = "qos")]
impl HeartbeatConsumer for NoopHeartbeatConsumer {
    fn send_heartbeat(
        &self,
        _status: &blueprint_qos::heartbeat::HeartbeatStatus,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = blueprint_qos::error::Result<()>> + Send + 'static>>
    {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::main]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

    // Optionally start QoS background service (metrics collection + on-chain reporting)
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

            match QoSServiceBuilder::<NoopHeartbeatConsumer>::new()
                .with_metrics_config(MetricsConfig::default())
                .with_dry_run(dry_run)
                .build()
                .await
            {
                Ok(qos_service) => {
                    info!("QoS service initialized (metrics_interval={metrics_interval}s, dry_run={dry_run})");

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
