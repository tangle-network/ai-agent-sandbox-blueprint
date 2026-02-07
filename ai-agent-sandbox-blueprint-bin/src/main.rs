//! Blueprint runner for ai-agent-sandbox-blueprint.

use ai_agent_sandbox_blueprint_lib::{JOB_WORKFLOW_TICK, bootstrap_workflows_from_chain, router};
use blueprint_producers_extra::cron::CronJob;
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::{TangleConsumer, TangleProducer};
use blueprint_sdk::{error, info};

#[tokio::main]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

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
    let tangle_config = TangleConfig::default();
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
