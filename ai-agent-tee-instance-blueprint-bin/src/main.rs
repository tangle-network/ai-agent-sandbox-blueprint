//! Blueprint runner for ai-agent-tee-instance-blueprint.
//!
//! TEE-backed variant: reads `TEE_BACKEND` env var to select the backend,
//! with direct lifecycle reporting and local TEE-backed provisioning. Supports Phala, AWS Nitro,
//! GCP Confidential Space, Azure SKR, and direct operator hardware.

use ai_agent_tee_instance_blueprint_lib::{
    JOB_WORKFLOW_TICK, bootstrap_workflows_from_chain, init_tee_backend,
    spawn_pending_provision_report_worker, tee_router, workflow_runtime_status_for_owner,
};
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router as HttpRouter};
use blueprint_producers_extra::cron::CronJob;
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::{TangleConsumer, TangleProducer};
use blueprint_sdk::{error, info, warn};

fn workflow_status_error(
    error: ai_agent_tee_instance_blueprint_lib::WorkflowStatusError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &error {
        ai_agent_tee_instance_blueprint_lib::WorkflowStatusError::NotFound(_) => {
            StatusCode::NOT_FOUND
        }
        ai_agent_tee_instance_blueprint_lib::WorkflowStatusError::Forbidden(_) => {
            StatusCode::FORBIDDEN
        }
        ai_agent_tee_instance_blueprint_lib::WorkflowStatusError::Internal(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
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
    Json<ai_agent_tee_instance_blueprint_lib::WorkflowRuntimeStatus>,
    (StatusCode, Json<serde_json::Value>),
> {
    workflow_runtime_status_for_owner(workflow_id, caller.as_str())
        .map(Json)
        .map_err(workflow_status_error)
}

async fn workflow_list_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    ai_agent_tee_instance_blueprint_lib::list_workflows_for_owner(caller.as_str())
        .map(|workflows| {
            Json(serde_json::json!({
                "workflows": workflows
                    .into_iter()
                    .map(|workflow| serde_json::json!({
                        "scope": "tee",
                        "workflowId": workflow.workflow_id,
                        "name": workflow.name,
                        "triggerType": workflow.trigger_type,
                        "triggerConfig": workflow.trigger_config,
                        "targetKind": workflow.target_kind,
                        "targetSandboxId": workflow.target_sandbox_id,
                        "targetServiceId": workflow.target_service_id,
                        "active": workflow.active,
                        "targetStatus": workflow.target_status,
                        "runnable": workflow.runnable,
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
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    ai_agent_tee_instance_blueprint_lib::workflow_detail_for_owner(workflow_id, caller.as_str())
        .map(|workflow| {
            Json(serde_json::json!({
                "scope": "tee",
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
                "targetStatus": workflow.target_status,
                "runnable": workflow.runnable,
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
        .route(
            "/api/workflows/{workflow_id}/detail",
            get(workflow_detail_handler),
        )
}

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

    if let Err(err) = bootstrap_workflows_from_chain(&tangle_client, service_id).await {
        error!("Failed to load workflows from chain: {err}");
    }

    // Reconcile stored sandbox state with Docker reality.
    ai_agent_tee_instance_blueprint_lib::reaper::reconcile_on_startup().await;

    // Start operator API for read-only operations (exec, prompt, task, ssh, snapshot).
    // TEE instance includes sealed-secrets endpoints.
    let api_port: u16 = std::env::var("OPERATOR_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);

    let tee_for_api = ai_agent_tee_instance_blueprint_lib::tee_backend()
        .map_err(|e| blueprint_sdk::Error::Other(format!("TEE backend not available: {e}")))?
        .clone();
    let api_shutdown = tokio::sync::watch::channel(());
    let api_shutdown_tx = api_shutdown.0;
    let api_handle = {
        let router = sandbox_runtime::operator_api::operator_api_router_with_tee_and_routes(
            Some(tee_for_api),
            workflow_status_router(),
        );
        let bind_all = std::env::var("BIND_ALL_INTERFACES")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let bind_ip: [u8; 4] = if bind_all {
            warn!(
                "BIND_ALL_INTERFACES=true — operator API is accessible on all network interfaces"
            );
            [0, 0, 0, 0]
        } else {
            [127, 0, 0, 1]
        };
        let addr = std::net::SocketAddr::from((bind_ip, api_port));
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
        ai_agent_tee_instance_blueprint_lib::auto_provision::AutoProvisionConfig::from_env(
            service_id,
        ) {
        info!("Auto-provision enabled (BSM={})", ap_config.bsm_address);
        let tee = ai_agent_tee_instance_blueprint_lib::tee_backend()
            .map_err(|e| blueprint_sdk::Error::Other(format!("TEE backend not available: {e}")))?;
        let report_client = tangle_client.clone();
        Some(tokio::spawn(async move {
            match ai_agent_tee_instance_blueprint_lib::auto_provision::run_auto_provision(
                ap_config,
                Some(tee.as_ref()),
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
        let config = ai_agent_tee_instance_blueprint_lib::runtime::SidecarRuntimeConfig::load();
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
                            ai_agent_tee_instance_blueprint_lib::reaper::reaper_tick()
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
                            ai_agent_tee_instance_blueprint_lib::reaper::gc_tick()
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
    #[cfg(feature = "billing")]
    let billing_shutdown_tx: Option<tokio::sync::broadcast::Sender<()>> = {
        let blueprint_id: u64 = std::env::var("BLUEPRINT_ID")
            .or_else(|_| std::env::var("TANGLE_BLUEPRINT_ID"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        if let Some(watchdog_config) =
            ai_agent_tee_instance_blueprint_lib::billing::EscrowWatchdogConfig::from_env(
                service_id,
                blueprint_id,
            )
        {
            if let Err(e) = watchdog_config.validate() {
                error!("Escrow watchdog config invalid: {e}");
                None
            } else {
                let (shutdown_tx, watchdog_rx) = tokio::sync::broadcast::channel::<()>(1);

                let tangle_contract = watchdog_config.tangle_contract;
                let report_client = tangle_client.clone();
                ai_agent_tee_instance_blueprint_lib::billing::spawn_watchdog(
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
        .router(tee_router())
        .producer(tangle_producer)
        .producer(workflow_cron)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async move {
            info!("Shutting down ai-agent-tee-instance-blueprint");

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
            info!("Shutdown complete; TEE instance lifecycle is report-driven");
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
