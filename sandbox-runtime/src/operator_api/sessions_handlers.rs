//! Extracted from operator_api.rs — sessions_handlers route group.

use super::*;

pub(crate) async fn sandbox_terminal_session_create_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    req: Option<Json<CreateLiveTerminalSessionRequest>>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let req = req.map(|Json(body)| body).unwrap_or_default();
    let summary = create_terminal_session(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

pub(crate) async fn sandbox_terminal_session_list_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let sessions = list_terminal_sessions(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

pub(crate) async fn sandbox_terminal_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    stream_terminal_session(&record, &session_id).await
}

pub(crate) async fn sandbox_terminal_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = delete_terminal_session(&record, &session_id).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn sandbox_terminal_session_resize_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
    Json(req): Json<TerminalResizeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    resize_terminal_session_on_sidecar(&record, &session_id, req.cols, req.rows).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "success": true }))))
}

pub(crate) async fn sandbox_terminal_session_input_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
    Json(req): Json<TerminalInputApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_sandbox(&sandbox_id, &address)?;
    send_terminal_input_to_sidecar(&record, &session_id, &req.data).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "success": true }))))
}

pub(crate) async fn sandbox_chat_session_create_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
    Json(req): Json<CreateLiveChatSessionRequest>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    require_running(&record)?;
    let summary = create_chat_session(live_scope_sandbox(&record.id), &address, req.title)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

pub(crate) async fn sandbox_chat_session_list_handler(
    SessionAuth(address): SessionAuth,
    Path(sandbox_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let sessions = list_chat_sessions(&live_scope_sandbox(&record.id), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

pub(crate) async fn sandbox_chat_session_get_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let detail = get_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(detail)))
}

pub(crate) async fn sandbox_chat_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    stream_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)
}

pub(crate) async fn sandbox_chat_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = delete_chat_session(&live_scope_sandbox(&record.id), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn sandbox_chat_run_cancel_handler(
    SessionAuth(address): SessionAuth,
    Path((sandbox_id, session_id, run_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let record = resolve_sandbox(&sandbox_id, &address)?;
    let resp = cancel_chat_run(
        &record,
        &live_scope_sandbox(&record.id),
        &address,
        &session_id,
        &run_id,
    )
    .await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_terminal_session_create_handler(
    SessionAuth(address): SessionAuth,
    req: Option<Json<CreateLiveTerminalSessionRequest>>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let req = req.map(|Json(body)| body).unwrap_or_default();
    let summary = create_terminal_session(&record, &req).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

pub(crate) async fn instance_terminal_session_list_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let sessions = list_terminal_sessions(&record).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

pub(crate) async fn instance_terminal_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    stream_terminal_session(&record, &session_id).await
}

pub(crate) async fn instance_terminal_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = delete_terminal_session(&record, &session_id).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_terminal_session_resize_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
    Json(req): Json<TerminalResizeApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    resize_terminal_session_on_sidecar(&record, &session_id, req.cols, req.rows).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "success": true }))))
}

pub(crate) async fn instance_terminal_session_input_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
    Json(req): Json<TerminalInputApiRequest>,
) -> impl IntoResponse {
    req.validate()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let record = resolve_instance(&address)?;
    send_terminal_input_to_sidecar(&record, &session_id, &req.data).await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "success": true }))))
}

pub(crate) async fn instance_chat_session_create_handler(
    SessionAuth(address): SessionAuth,
    Json(req): Json<CreateLiveChatSessionRequest>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    require_running(&record)?;
    let summary = create_chat_session(live_scope_instance(&record), &address, req.title)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(summary)))
}

pub(crate) async fn instance_chat_session_list_handler(
    SessionAuth(address): SessionAuth,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let sessions = list_chat_sessions(&live_scope_instance(&record), &address)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(json!({ "sessions": sessions }))))
}

pub(crate) async fn instance_chat_session_get_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let detail = get_chat_session(&live_scope_instance(&record), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(detail)))
}

pub(crate) async fn instance_chat_session_stream_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    stream_chat_session(&live_scope_instance(&record), &address, &session_id)
}

pub(crate) async fn instance_chat_session_delete_handler(
    SessionAuth(address): SessionAuth,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = delete_chat_session(&live_scope_instance(&record), &address, &session_id)?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}

pub(crate) async fn instance_chat_run_cancel_handler(
    SessionAuth(address): SessionAuth,
    Path((session_id, run_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let record = resolve_instance(&address)?;
    let resp = cancel_chat_run(
        &record,
        &live_scope_instance(&record),
        &address,
        &session_id,
        &run_id,
    )
    .await?;
    Ok::<_, (StatusCode, Json<ApiError>)>((StatusCode::OK, Json(resp)))
}
