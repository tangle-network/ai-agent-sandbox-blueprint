//! Blueprint runner for ai-agent-tee-instance-blueprint.
//!
//! TEE-backed variant: reads `TEE_BACKEND` env var to select the backend,
//! then routes provision/deprovision through it. Supports Phala, AWS Nitro,
//! GCP Confidential Space, Azure SKR, and direct operator hardware.

use ai_agent_tee_instance_blueprint_lib::{init_tee_backend, tee_router};
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::{TangleConsumer, TangleProducer};
use blueprint_sdk::{error, info};

#[tokio::main]
#[allow(clippy::result_large_err)]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

    // ── TEE backend ──────────────────────────────────────────────────────
    let backend = sandbox_runtime::tee::backend_factory::backend_from_env()
        .map_err(|e| blueprint_sdk::Error::Other(format!("Failed to create TEE backend: {e}")))?;
    let backend_type = format!("{:?}", backend.tee_type());
    init_tee_backend(backend);
    info!("TEE backend initialized (type: {backend_type})");

    // ── Tangle setup ─────────────────────────────────────────────────────
    let env = BlueprintEnvironment::load()?;

    let tangle_client = env
        .tangle_client()
        .await
        .map_err(|e| blueprint_sdk::Error::Other(e.to_string()))?;

    let service_id = env
        .protocol_settings
        .tangle()
        .map_err(|e| blueprint_sdk::Error::Other(e.to_string()))?
        .service_id
        .ok_or_else(|| blueprint_sdk::Error::Other("SERVICE_ID missing".into()))?;

    info!("Starting ai-agent-tee-instance-blueprint for service {service_id}");

    // Reconcile stored sandbox state with Docker reality.
    ai_agent_tee_instance_blueprint_lib::reaper::reconcile_on_startup().await;

    // Spawn reaper background task (idle timeout + max lifetime enforcement).
    {
        let config = ai_agent_tee_instance_blueprint_lib::runtime::SidecarRuntimeConfig::load();
        let reaper_interval = config.sandbox_reaper_interval;

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(reaper_interval));
            loop {
                interval.tick().await;
                ai_agent_tee_instance_blueprint_lib::reaper::reaper_tick().await;
            }
        });
    }

    let tangle_producer = TangleProducer::new(tangle_client.clone(), service_id);
    let tangle_consumer = TangleConsumer::new(tangle_client);

    let tangle_config = TangleConfig::default();

    let result = BlueprintRunner::builder(tangle_config, env)
        .router(tee_router())
        .producer(tangle_producer)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async {
            info!("Shutting down ai-agent-tee-instance-blueprint");
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
