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

    // Auto-provision: read service config from BSM and provision sandbox on startup.
    if let Some(ap_config) =
        ai_agent_tee_instance_blueprint_lib::auto_provision::AutoProvisionConfig::from_env(service_id)
    {
        info!("Auto-provision enabled (BSM={})", ap_config.bsm_address);
        let tee = ai_agent_tee_instance_blueprint_lib::tee_backend();
        tokio::spawn(async move {
            match ai_agent_tee_instance_blueprint_lib::auto_provision::run_auto_provision(
                ap_config,
                Some(tee.as_ref()),
            )
            .await
            {
                Ok(()) => info!("Auto-provision completed"),
                Err(e) => error!("Auto-provision failed: {e}"),
            }
        });
    }

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

    // Spawn escrow watchdog + subscription billing keeper.
    #[cfg(feature = "billing")]
    let billing_shutdown_tx: Option<tokio::sync::broadcast::Sender<()>> = {
        let blueprint_id: u64 = std::env::var("BLUEPRINT_ID")
            .or_else(|_| std::env::var("TANGLE_BLUEPRINT_ID"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        if let Some(watchdog_config) =
            ai_agent_tee_instance_blueprint_lib::billing::EscrowWatchdogConfig::from_env(
                service_id, blueprint_id,
            )
        {
            if let Err(e) = watchdog_config.validate() {
                error!("Escrow watchdog config invalid: {e}");
                None
            } else {
                let (shutdown_tx, watchdog_rx) = tokio::sync::broadcast::channel::<()>(1);

                let tangle_contract = watchdog_config.tangle_contract;
                ai_agent_tee_instance_blueprint_lib::billing::spawn_watchdog(
                    watchdog_config, watchdog_rx,
                );
                info!("Escrow watchdog started for service {service_id}");

                // Subscription billing keeper: calls billSubscriptionBatch on-chain.
                let keystore = std::sync::Arc::new(env.keystore());
                let rpc_endpoint = env.http_rpc_endpoint.to_string();

                let billing_check_secs: u64 = std::env::var("BILLING_CHECK_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60);
                let billing_rescan_secs: u64 = std::env::var("BILLING_RESCAN_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300);

                use blueprint_tangle_extra::services::{
                    BackgroundKeeper, KeeperConfig, SubscriptionBillingKeeper,
                };

                let keeper_config = KeeperConfig::new(rpc_endpoint, keystore)
                    .with_tangle_contract(tangle_contract)
                    .with_billing_interval(std::time::Duration::from_secs(billing_check_secs))
                    .with_billing_rescan_interval(std::time::Duration::from_secs(
                        billing_rescan_secs,
                    ));

                let billing_rx = shutdown_tx.subscribe();
                let _billing_handle =
                    SubscriptionBillingKeeper::start(keeper_config, billing_rx);

                info!("Subscription billing keeper started for service {service_id}");
                Some(shutdown_tx)
            }
        } else {
            None
        }
    };

    let tangle_producer = TangleProducer::new(tangle_client.clone(), service_id);
    let tangle_consumer = TangleConsumer::new(tangle_client);

    let tangle_config = TangleConfig::default();

    let result = BlueprintRunner::builder(tangle_config, env)
        .router(tee_router())
        .producer(tangle_producer)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async move {
            info!("Shutting down ai-agent-tee-instance-blueprint");

            #[cfg(feature = "billing")]
            if let Some(tx) = billing_shutdown_tx {
                let _ = tx.send(());
                info!("Billing shutdown signal sent");
            }

            // Deprovision sandbox + TEE deployment so they don't outlive the service.
            let tee = ai_agent_tee_instance_blueprint_lib::tee_backend();
            match ai_agent_tee_instance_blueprint_lib::deprovision_core(Some(tee.as_ref())).await {
                Ok((_, id)) => info!("Shutdown: deprovisioned sandbox {id}"),
                Err(e) => info!("Shutdown: no sandbox to deprovision ({e})"),
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
