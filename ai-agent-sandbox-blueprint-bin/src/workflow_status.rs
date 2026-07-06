//! Workflow-status HTTP endpoints (status / list / detail) + their router.

use super::*;

pub(crate) fn workflow_status_error(
    error: WorkflowStatusError,
) -> (StatusCode, Json<serde_json::Value>) {
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

pub(crate) async fn workflow_status_handler(
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

pub(crate) async fn workflow_list_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
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

pub(crate) async fn workflow_detail_handler(
    sandbox_runtime::session_auth::SessionAuth(caller): sandbox_runtime::session_auth::SessionAuth,
    Path(workflow_id): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
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

pub(crate) fn workflow_status_router() -> HttpRouter {
    HttpRouter::new()
        .route("/api/workflows", get(workflow_list_handler))
        .route("/api/workflows/{workflow_id}", get(workflow_status_handler))
        .route(
            "/api/workflows/{workflow_id}/detail",
            get(workflow_detail_handler),
        )
}
