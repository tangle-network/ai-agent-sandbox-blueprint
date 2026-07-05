//! Extracted from operator_api.rs — chat_handlers route group.

use super::*;

pub(crate) fn accepted_prompt_response(run: &ChatRunRecord, session_id: &str) -> PromptApiResponse {
    PromptApiResponse {
        accepted: true,
        run_id: run.id.clone(),
        session_id: session_id.to_string(),
        status: chat_run_status_label(&run.status).to_string(),
        accepted_at: run.created_at,
    }
}

pub(crate) fn accepted_task_response(run: &ChatRunRecord, session_id: &str) -> TaskApiResponse {
    TaskApiResponse {
        accepted: true,
        run_id: run.id.clone(),
        session_id: session_id.to_string(),
        status: chat_run_status_label(&run.status).to_string(),
        accepted_at: run.created_at,
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────

pub(crate) async fn sandbox_prompt_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let scope = live_scope_sandbox(&record.id);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Prompt,
        &req.message,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.message,
            backend_type: req.backend_type,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: None,
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_prompt_response(&run, &session.id)),
    ))
}

pub(crate) async fn instance_prompt_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<PromptApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let scope = live_scope_instance(&record);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Prompt,
        &req.message,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.message,
            backend_type: req.backend_type,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: None,
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_prompt_response(&run, &session.id)),
    ))
}

// ── Task ─────────────────────────────────────────────────────────────────

pub(crate) async fn sandbox_task_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let scope = live_scope_sandbox(&record.id);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Task,
        &req.prompt,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.prompt,
            backend_type: req.backend_type,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: Some(req.max_turns),
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_task_response(&run, &session.id)),
    ))
}

pub(crate) async fn instance_task_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<TaskApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    let scope = live_scope_instance(&record);
    require_running(&record)?;
    let (session, run) = enqueue_chat_run(
        &scope,
        &address,
        &req.session_id,
        ChatRunKind::Task,
        &req.prompt,
    )?;
    spawn_chat_run(
        record,
        SpawnChatRunRequest {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            message: req.prompt,
            backend_type: req.backend_type,
            model: req.model,
            context_json: req.context_json,
            timeout_ms: req.timeout_ms,
            max_turns: Some(req.max_turns),
        },
    );
    Ok::<_, (StatusCode, Json<ApiError>)>((
        StatusCode::ACCEPTED,
        Json(accepted_task_response(&run, &session.id)),
    ))
}
