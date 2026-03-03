//! Blueprint runner for ai-agent-instance-blueprint.
//!
//! Subscription model: each service instance runs exactly one sandbox.
//! Simpler than the multi-sandbox blueprint — singleton lifecycle + workflows.

use ai_agent_instance_blueprint_lib::{
    JOB_WORKFLOW_TICK, bootstrap_workflows_from_chain, router,
    spawn_pending_provision_report_worker,
};
use blueprint_producers_extra::cron::CronJob;
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::{TangleConsumer, TangleProducer};
use blueprint_sdk::{error, info, warn};

#[tokio::main]
#[allow(clippy::result_large_err)]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

    // Validate required auth config — SESSION_AUTH_SECRET must be set in production.
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

    info!("Starting ai-agent-instance-blueprint for service {service_id}");

    if let Err(err) = bootstrap_workflows_from_chain(&tangle_client, service_id).await {
        error!("Failed to load workflows from chain: {err}");
    }

    // Reconcile stored sandbox state with Docker reality.
    ai_agent_instance_blueprint_lib::reaper::reconcile_on_startup().await;

    // Start operator API for read-only operations (exec, prompt, task, ssh, snapshot).
    // Instance mode uses singleton /api/sandbox/* endpoints.
    let api_port: u16 = std::env::var("OPERATOR_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);

    let api_shutdown = tokio::sync::watch::channel(());
    let api_shutdown_tx = api_shutdown.0;
    let api_handle = {
        let router = sandbox_runtime::operator_api::operator_api_router();
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0u8], api_port));
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

    // Retry any pending direct lifecycle reports left by transient RPC/tx failures.
    let _pending_report_handle = spawn_pending_provision_report_worker(
        tangle_client.clone(),
        service_id,
        api_shutdown_tx.subscribe(),
    );

    // Auto-provision: read service config from BSM and provision sandbox on startup.
    // Track the JoinHandle so we can abort it during shutdown if it's still running.
    let auto_provision_handle: Option<tokio::task::JoinHandle<()>> = if let Some(ap_config) =
        ai_agent_instance_blueprint_lib::auto_provision::AutoProvisionConfig::from_env(service_id)
    {
        info!("Auto-provision enabled (BSM={})", ap_config.bsm_address);
        let report_client = tangle_client.clone();
        Some(tokio::spawn(async move {
            match ai_agent_instance_blueprint_lib::auto_provision::run_auto_provision(
                ap_config,
                None,
                Some(report_client),
            )
            .await
            {
                Ok(()) => info!("Auto-provision completed"),
                Err(e) => error!("Auto-provision failed: {e}"),
            }
        }))
    } else {
        None
    };

    // Spawn reaper background task (idle timeout + max lifetime enforcement).
    {
        let config = ai_agent_instance_blueprint_lib::runtime::SidecarRuntimeConfig::load();
        let reaper_interval = config.sandbox_reaper_interval;
        let gc_interval = config.sandbox_gc_interval;

        let mut reaper_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(reaper_interval));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let h = tokio::spawn(
                            ai_agent_instance_blueprint_lib::reaper::reaper_tick()
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

        // Spawn GC background task (stopped sandbox cleanup — images, committed snapshots)
        let mut gc_shutdown = api_shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(gc_interval));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let h = tokio::spawn(
                            ai_agent_instance_blueprint_lib::reaper::gc_tick()
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

    // Spawn escrow watchdog + subscription billing keeper.
    // Only active when TANGLE_CONTRACT_ADDRESS is set (billing feature enabled at build time).
    #[cfg(feature = "billing")]
    let billing_shutdown_tx: Option<tokio::sync::broadcast::Sender<()>> = {
        let blueprint_id: u64 = std::env::var("BLUEPRINT_ID")
            .or_else(|_| std::env::var("TANGLE_BLUEPRINT_ID"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        if let Some(watchdog_config) =
            ai_agent_instance_blueprint_lib::billing::EscrowWatchdogConfig::from_env(
                service_id,
                blueprint_id,
            )
        {
            if let Err(e) = watchdog_config.validate() {
                error!("Escrow watchdog config invalid: {e}");
                None
            } else {
                let (shutdown_tx, watchdog_rx) = tokio::sync::broadcast::channel::<()>(1);

                // Escrow watchdog: auto-deprovision when escrow is exhausted.
                let tangle_contract = watchdog_config.tangle_contract;
                let report_client = tangle_client.clone();
                ai_agent_instance_blueprint_lib::billing::spawn_watchdog(
                    watchdog_config,
                    watchdog_rx,
                    Some(report_client),
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
                let _billing_handle = SubscriptionBillingKeeper::start(keeper_config, billing_rx);

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
    let cron_schedule =
        std::env::var("WORKFLOW_CRON_SCHEDULE").unwrap_or_else(|_| "0 * * * * *".to_string());
    let workflow_cron = CronJob::new(JOB_WORKFLOW_TICK, cron_schedule.as_str())
        .await
        .map_err(|err| blueprint_sdk::Error::Other(format!("Invalid workflow cron: {err}")))?;

    let result = BlueprintRunner::builder(tangle_config, env)
        .router(router())
        .producer(tangle_producer)
        .producer(workflow_cron)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async move {
            info!("Shutting down ai-agent-instance-blueprint");

            // Abort auto-provision if it's still running (e.g., blocked on RPC).
            if let Some(h) = auto_provision_handle {
                h.abort();
                info!("Auto-provision task aborted");
            }

            // Drain operator API first.
            drop(api_shutdown_tx);
            match tokio::time::timeout(std::time::Duration::from_secs(10), api_handle).await {
                Ok(Ok(())) => info!("Operator API shut down cleanly"),
                Ok(Err(e)) => error!("Operator API task panicked: {e}"),
                Err(_) => warn!("Operator API shutdown timed out after 10s"),
            }

            #[cfg(feature = "billing")]
            if let Some(tx) = billing_shutdown_tx {
                let _ = tx.send(());
                info!("Billing shutdown signal sent");
            }

            // Do not deprovision on generic process shutdown.
            // Lifecycle transitions are reported explicitly by operator logic.
            info!("Shutdown complete; instance lifecycle is report-driven");
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
