//! Extracted from operator_api.rs — sessions_core route group.

use super::*;

// ---------------------------------------------------------------------------
// Live chat / terminal session endpoints
// ---------------------------------------------------------------------------

pub(crate) fn parse_terminal_session_descriptor(
    value: &Value,
) -> Option<TerminalSessionDescriptor> {
    let session_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(Value::as_str)?
        .to_string();
    let title = value
        .get("title")
        .or_else(|| value.get("name"))
        .or_else(|| value.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let stream_path = value
        .get("streamUrl")
        .or_else(|| value.get("stream_url"))
        .and_then(Value::as_str)
        .map(str::to_string);

    Some(TerminalSessionDescriptor {
        session_id,
        title,
        stream_path,
    })
}

pub(crate) fn terminal_session_summary(
    descriptor: &TerminalSessionDescriptor,
) -> LiveSessionSummary {
    LiveSessionSummary {
        session_id: descriptor.session_id.clone(),
        title: descriptor.title.clone(),
        active_run_id: None,
    }
}

pub(crate) fn parse_terminal_session_summary(value: &Value) -> Option<LiveSessionSummary> {
    parse_terminal_session_descriptor(value).map(|descriptor| terminal_session_summary(&descriptor))
}

pub(crate) fn parse_terminal_session_response(
    parsed: &Value,
) -> Result<TerminalSessionDescriptor, (StatusCode, Json<ApiError>)> {
    parse_terminal_session_descriptor(parsed.get("data").ok_or_else(|| {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Missing sidecar terminal session data",
        )
    })?)
    .ok_or_else(|| {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Invalid sidecar terminal session response",
        )
    })
}

pub(crate) async fn fetch_terminal_session_descriptor(
    record: &SandboxRecord,
    session_id: &str,
) -> Result<TerminalSessionDescriptor, (StatusCode, Json<ApiError>)> {
    let parsed = terminal_sidecar_get_call(
        record,
        &format!("/terminals/{session_id}"),
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal detail",
    )
    .await?;
    parse_terminal_session_response(&parsed)
}

pub(crate) async fn resolve_terminal_stream_path(
    record: &SandboxRecord,
    session_id: &str,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    let descriptor = fetch_terminal_session_descriptor(record, session_id).await?;
    Ok(descriptor
        .stream_path
        .unwrap_or_else(|| format!("/terminals/{session_id}/stream")))
}

pub(crate) async fn create_terminal_session(
    record: &SandboxRecord,
    req: &CreateLiveTerminalSessionRequest,
) -> Result<LiveSessionSummary, (StatusCode, Json<ApiError>)> {
    require_running(record)?;
    let mut payload = Map::new();
    payload.insert(
        "env".into(),
        json!({
            "PS1": TERMINAL_PROMPT,
            "PROMPT_DIRTRIM": "0",
        }),
    );
    let cwd = req.cwd.trim();
    if !cwd.is_empty() {
        payload.insert("cwd".into(), json!(cwd));
    }
    if let Some(cols) = req.cols {
        payload.insert("cols".into(), json!(cols));
    }
    if let Some(rows) = req.rows {
        payload.insert("rows".into(), json!(rows));
    }
    let parsed = terminal_sidecar_call(
        record,
        "/terminals",
        Value::Object(payload),
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal create",
        true,
    )
    .await?;
    Ok(terminal_session_summary(&parse_terminal_session_response(
        &parsed,
    )?))
}

pub(crate) async fn list_terminal_sessions(
    record: &SandboxRecord,
) -> Result<Vec<LiveSessionSummary>, (StatusCode, Json<ApiError>)> {
    let parsed = terminal_sidecar_get_call(
        record,
        "/terminals",
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal list",
    )
    .await?;
    Ok(parsed
        .get("data")
        .and_then(Value::as_array)
        .map(|sessions| {
            sessions
                .iter()
                .filter_map(parse_terminal_session_summary)
                .collect()
        })
        .unwrap_or_default())
}

pub(crate) async fn stream_terminal_session(
    record: &SandboxRecord,
    session_id: &str,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let stream_path = resolve_terminal_stream_path(record, session_id).await?;
    let response = terminal_sidecar_stream_call(
        record,
        &stream_path,
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal stream",
    )
    .await?;

    let mut proxied = axum::response::Response::new(Body::from_stream(
        response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other)),
    ));
    *proxied.status_mut() = StatusCode::OK;
    proxied.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(proxied)
}

pub(crate) async fn delete_terminal_session(
    record: &SandboxRecord,
    session_id: &str,
) -> Result<serde_json::Value, (StatusCode, Json<ApiError>)> {
    terminal_sidecar_delete_call(
        record,
        &format!("/terminals/{session_id}"),
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal delete",
    )
    .await?;

    Ok(json!({ "deleted": true, "session_id": session_id }))
}

pub(crate) async fn send_terminal_input_to_sidecar(
    record: &SandboxRecord,
    session_id: &str,
    data: &str,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    terminal_sidecar_call(
        record,
        &format!("/terminals/{session_id}/input"),
        json!({ "data": data }),
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal input",
        true,
    )
    .await?;
    Ok(())
}

pub(crate) async fn resize_terminal_session_on_sidecar(
    record: &SandboxRecord,
    session_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    terminal_sidecar_patch_call(
        record,
        &format!("/terminals/{session_id}"),
        json!({
            "cols": cols,
            "rows": rows,
        }),
        SIDECAR_DEFAULT_TIMEOUT,
        "terminal resize",
    )
    .await?;
    Ok(())
}

pub(crate) fn create_chat_session(
    scope_id: String,
    owner: &str,
    title: String,
) -> Result<LiveSessionSummary, (StatusCode, Json<ApiError>)> {
    let session = chat_state::create_session(&scope_id, owner, Some(&title))
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(LiveSessionSummary {
        session_id: session.id,
        title: session.title,
        active_run_id: session.active_run_id,
    })
}

pub(crate) fn list_chat_sessions(
    scope_id: &str,
    owner: &str,
) -> Result<Vec<LiveSessionSummary>, (StatusCode, Json<ApiError>)> {
    chat_state::list_sessions(scope_id, owner)
        .map(|sessions| {
            sessions
                .into_iter()
                .map(|s| LiveSessionSummary {
                    session_id: s.id,
                    title: s.title,
                    active_run_id: s.active_run_id,
                })
                .collect()
        })
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub(crate) fn get_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<LiveChatSessionDetail, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    let runs = chat_state::list_runs_for_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let run_progress = chat_state::list_run_progress_for_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(LiveChatSessionDetail {
        session_id: session.id,
        title: session.title,
        sidecar_session_id: session.latest_sidecar_session_id,
        active_run_id: session.active_run_id,
        messages: session.messages,
        run_progress,
        runs,
    })
}

pub(crate) fn stream_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    let rx = chat_state::subscribe_events(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(sse_from_json_events(rx).into_response())
}

pub(crate) fn delete_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<serde_json::Value, (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }
    if let Some(active_run_id) = session.active_run_id.as_deref()
        && let Some(run) = chat_state::get_run(active_run_id)
            .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        && run.status.is_active()
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "Cannot delete a chat session while a run is active",
        ));
    }
    chat_state::delete_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(json!({ "deleted": true, "session_id": session_id }))
}

pub(crate) async fn cancel_chat_run(
    record: &SandboxRecord,
    scope_id: &str,
    owner: &str,
    session_id: &str,
    run_id: &str,
) -> Result<CancelChatRunResponse, (StatusCode, Json<ApiError>)> {
    let (session, run) = resolve_chat_run(scope_id, owner, session_id, run_id)?;

    if run.status == ChatRunStatus::Cancelled {
        return Ok(CancelChatRunResponse {
            success: true,
            session_id: session.id,
            run_id: run.id,
            status: chat_run_status_label(&run.status).to_string(),
            cancelled_at: run.completed_at.unwrap_or(run.created_at),
        });
    }

    if !run.status.is_active() {
        return Ok(CancelChatRunResponse {
            success: true,
            session_id: session.id,
            run_id: run.id,
            status: chat_run_status_label(&run.status).to_string(),
            cancelled_at: run.completed_at.unwrap_or(chat_state::now_ms()),
        });
    }

    if session.active_run_id.as_deref() != Some(run.id.as_str()) {
        return Err(api_error_with_details(
            StatusCode::CONFLICT,
            "This run is no longer the active chat run for the session",
            Some("CHAT_RUN_NOT_ACTIVE"),
            None,
        ));
    }

    let cancelling_at = chat_state::now_ms();
    let _ = chat_state::update_run(&run.id, |entry| {
        entry.status = ChatRunStatus::Cancelling;
        if entry.started_at.is_none() {
            entry.started_at = Some(cancelling_at);
        }
    });
    if let Ok(Some(cancelling_run)) = chat_state::get_run(&run.id) {
        publish_run_event(&session.id, "run_cancel_requested", &cancelling_run);
        publish_run_progress(
            &session.id,
            &cancelling_run.id,
            &cancelling_run.status,
            "cancelling",
            "Cancellation requested. Stopping the active run.",
        );
    }

    abort_chat_run_task(&run.id);
    let updated_run = finalize_cancelled_chat_run(&session.id, &run.id, "Run cancelled by user.")?;
    best_effort_cancel_sidecar_run(record).await;

    Ok(CancelChatRunResponse {
        success: true,
        session_id: session.id,
        run_id: updated_run.id,
        status: chat_run_status_label(&updated_run.status).to_string(),
        cancelled_at: updated_run.completed_at.unwrap_or(cancelling_at),
    })
}
