//! Extracted from operator_api.rs — chat route group.

use super::*;

pub(crate) fn chat_run_status_label(status: &ChatRunStatus) -> &'static str {
    match status {
        ChatRunStatus::Queued => "queued",
        ChatRunStatus::Running => "running",
        ChatRunStatus::Cancelling => "cancelling",
        ChatRunStatus::Completed => "completed",
        ChatRunStatus::Failed => "failed",
        ChatRunStatus::Cancelled => "cancelled",
        ChatRunStatus::Interrupted => "interrupted",
    }
}

pub(crate) fn resolve_agent_run_timeout_ms(timeout_ms: u64, max_turns: Option<u64>) -> u64 {
    if timeout_ms > 0 {
        timeout_ms
    } else if max_turns.is_some() {
        DEFAULT_TASK_RUN_TIMEOUT_MS
    } else {
        DEFAULT_PROMPT_RUN_TIMEOUT_MS
    }
}

pub(crate) fn resolve_or_create_chat_session(
    scope_id: &str,
    owner: &str,
    session_id: &str,
) -> Result<ChatSessionRecord, (StatusCode, Json<ApiError>)> {
    if session_id.trim().is_empty() {
        return chat_state::create_session(scope_id, owner, Some("New Chat"))
            .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e));
    }

    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;

    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }

    Ok(session)
}

pub(crate) fn resolve_chat_run(
    scope_id: &str,
    owner: &str,
    session_id: &str,
    run_id: &str,
) -> Result<(ChatSessionRecord, ChatRunRecord), (StatusCode, Json<ApiError>)> {
    let session = chat_state::get_session(session_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Chat session not found"))?;
    if !chat_session_matches(&session, scope_id, owner) {
        return Err(api_error(StatusCode::NOT_FOUND, "Chat session not found"));
    }

    let run = chat_state::get_run(run_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            api_error_with_details(
                StatusCode::NOT_FOUND,
                "Chat run not found",
                Some("CHAT_RUN_NOT_FOUND"),
                None,
            )
        })?;

    if run.session_id != session.id
        || run.scope_id != scope_id
        || !run.owner.eq_ignore_ascii_case(owner)
    {
        return Err(api_error_with_details(
            StatusCode::NOT_FOUND,
            "Chat run not found",
            Some("CHAT_RUN_NOT_FOUND"),
            None,
        ));
    }

    Ok((session, run))
}

pub(crate) async fn best_effort_cancel_sidecar_run(record: &SandboxRecord) {
    let _ = tokio::time::timeout(
        CHAT_CANCEL_TIMEOUT,
        sidecar_post_json(
            &record.sidecar_url,
            "/agents/run/cancel",
            &record.token,
            json!({}),
        ),
    )
    .await;
}

pub(crate) fn finalize_cancelled_chat_run(
    session_id: &str,
    run_id: &str,
    error_text: &str,
) -> Result<ChatRunRecord, (StatusCode, Json<ApiError>)> {
    let cancelled_at = chat_state::now_ms();
    let updated = chat_state::update_run(run_id, |run| {
        run.status = ChatRunStatus::Cancelled;
        run.completed_at = Some(cancelled_at);
        run.error = Some(error_text.to_string());
    })
    .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if !updated {
        return Err(api_error_with_details(
            StatusCode::NOT_FOUND,
            "Chat run not found",
            Some("CHAT_RUN_NOT_FOUND"),
            None,
        ));
    }

    let _ = chat_state::clear_session_active_run(session_id);
    let updated_run = chat_state::get_run(run_id)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Chat run disappeared"))?;
    publish_run_event(session_id, "run_cancelled", &updated_run);
    publish_run_progress(
        session_id,
        &updated_run.id,
        &updated_run.status,
        "cancelled",
        "Run cancelled by user.",
    );
    emit_session_error(
        session_id,
        "Execution cancelled by user",
        Some("EXECUTION_CANCELLED"),
    );
    emit_session_idle(session_id);
    Ok(updated_run)
}

pub(crate) fn enqueue_chat_run(
    scope_id: &str,
    owner: &str,
    session_id: &str,
    kind: ChatRunKind,
    request_text: &str,
) -> Result<(ChatSessionRecord, ChatRunRecord), (StatusCode, Json<ApiError>)> {
    let _guard = CHAT_RUN_ENQUEUE_GUARD.lock().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chat enqueue lock poisoned: {e}"),
        )
    })?;
    if let Some(existing) = chat_state::active_run_for_scope(scope_id, owner)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(api_error_with_details(
            StatusCode::CONFLICT,
            format!(
                "A chat run is already active for this resource ({})",
                existing.id
            ),
            Some("CHAT_RUN_ACTIVE"),
            None,
        ));
    }

    let session = resolve_or_create_chat_session(scope_id, owner, session_id)?;
    let run = chat_state::create_run(&session.id, scope_id, owner, kind, request_text)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let _ = chat_state::maybe_auto_title_session(&session.id, request_text);

    let user_message = ChatMessageRecord {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: Some(run.id.clone()),
        role: "user".to_string(),
        content: request_text.to_string(),
        created_at: chat_state::now_ms(),
        completed_at: Some(chat_state::now_ms()),
        parts: vec![json!({
            "id": format!("text-{}", uuid::Uuid::new_v4()),
            "type": "text",
            "text": request_text.to_string(),
        })],
        trace_id: None,
        success: None,
        error: None,
    };
    publish_chat_message(&session.id, user_message, "user_message");
    if let Ok(Some(current_session)) = chat_state::get_session(&session.id)
        && let Some(message) = current_session.messages.last()
    {
        emit_message_updated(&session.id, message);
        for part in &message.parts {
            emit_message_part_updated(&session.id, &message.id, part.clone());
        }
    }
    if let Ok(Some(queued_run)) = chat_state::get_run(&run.id) {
        publish_run_event(&session.id, "run_queued", &queued_run);
        return Ok((session, queued_run));
    }

    Ok((session, run))
}

pub(crate) struct SpawnChatRunRequest {
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) message: String,
    pub(crate) backend_type: String,
    pub(crate) model: String,
    pub(crate) context_json: String,
    pub(crate) timeout_ms: u64,
    pub(crate) max_turns: Option<u64>,
}

pub(crate) fn spawn_chat_run(record: SandboxRecord, request: SpawnChatRunRequest) {
    let SpawnChatRunRequest {
        session_id,
        run_id,
        message,
        backend_type,
        model,
        context_json,
        timeout_ms,
        max_turns,
    } = request;
    let spawned_run_id = run_id.clone();
    let handle = tokio::spawn(async move {
        struct ChatRunAbortGuard {
            run_id: String,
        }

        impl Drop for ChatRunAbortGuard {
            fn drop(&mut self) {
                clear_chat_run_abort(&self.run_id);
            }
        }

        let _abort_guard = ChatRunAbortGuard {
            run_id: run_id.clone(),
        };
        publish_run_progress(
            &session_id,
            &run_id,
            &ChatRunStatus::Queued,
            "queued",
            "Run accepted and queued by the operator.",
        );

        let started_at = chat_state::now_ms();
        let _ = chat_state::update_run(&run_id, |run| {
            run.status = ChatRunStatus::Running;
            run.started_at = Some(started_at);
        });
        if let Ok(Some(run)) = chat_state::get_run(&run_id) {
            publish_run_event(&session_id, "run_started", &run);
            publish_run_progress(
                &session_id,
                &run_id,
                &run.status,
                "running",
                "Operator started the agent run.",
            );
        }

        let sidecar_session_id = chat_state::get_session(&session_id)
            .ok()
            .flatten()
            .and_then(|session| session.latest_sidecar_session_id)
            .unwrap_or_default();

        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let assistant_started_at = chat_state::now_ms();
        let assistant_message = ChatMessageRecord {
            id: assistant_message_id.clone(),
            run_id: Some(run_id.clone()),
            role: "assistant".to_string(),
            content: String::new(),
            created_at: assistant_started_at,
            completed_at: None,
            parts: Vec::new(),
            trace_id: None,
            success: None,
            error: None,
        };
        let _ = chat_state::append_message(&session_id, assistant_message.clone());
        emit_message_updated(&session_id, &assistant_message);
        let mut ignored_upstream_message_ids = HashSet::new();
        let mut assistant_upstream_message_ids = HashSet::new();
        let mut authoritative_sidecar_session_id =
            (!sidecar_session_id.trim().is_empty()).then(|| sidecar_session_id.clone());
        let mut authoritative_sidecar_session_source = if authoritative_sidecar_session_id.is_some()
        {
            LiveChatSidecarSessionSource::Request
        } else {
            LiveChatSidecarSessionSource::None
        };

        let result = agent_stream_on_sidecar(
            &record,
            AgentStreamRequest {
                message: &message,
                session_id: &sidecar_session_id,
                backend_type: &backend_type,
                model: &model,
                context_json: &context_json,
                timeout_ms,
                max_turns,
            },
            |event| {
                let streamed_session = match event.event_type.as_str() {
                    // execution.started carries the reusable sidecar session ID for
                    // this live-chat flow. Later result metadata may use a different
                    // backend-specific namespace, so do not let it override this.
                    "execution.started" => extract_stream_session_id(&event.data)
                        .map(|value| (value, LiveChatSidecarSessionSource::ExecutionStarted)),
                    "session.updated" => extract_stream_session_id(&event.data)
                        .map(|value| (value, LiveChatSidecarSessionSource::SessionUpdated)),
                    _ => None,
                };

                if let Some((candidate_session_id, candidate_source)) = streamed_session
                    && candidate_source > authoritative_sidecar_session_source
                {
                    authoritative_sidecar_session_source = candidate_source;
                    authoritative_sidecar_session_id = Some(candidate_session_id.clone());
                    let _ = chat_state::set_session_sidecar_session_id(
                        &session_id,
                        Some(candidate_session_id.clone()),
                    );
                    let _ = chat_state::update_run(&run_id, |run| {
                        run.sidecar_session_id = Some(candidate_session_id.clone());
                    });
                }

                if event.event_type == "message.part.updated"
                    && let Some(part) = event.data.get("part").and_then(normalize_stream_part)
                {
                    if !should_forward_stream_part(
                        &part,
                        &message,
                        &mut ignored_upstream_message_ids,
                        &mut assistant_upstream_message_ids,
                    ) {
                        return;
                    }
                    let _ = chat_state::upsert_message_part(
                        &session_id,
                        &assistant_message_id,
                        part.clone(),
                    );
                    emit_message_part_updated(&session_id, &assistant_message_id, part);
                }
            },
        )
        .await;

        if let Ok(Some(existing_run)) = chat_state::get_run(&run_id)
            && matches!(
                existing_run.status,
                ChatRunStatus::Cancelled | ChatRunStatus::Cancelling
            )
        {
            return;
        }

        match result {
            Ok(ar) => {
                metrics::metrics().record_job(ar.duration_ms, ar.input_tokens, ar.output_tokens);
                let completed_at = chat_state::now_ms();
                let final_status = if ar.success {
                    ChatRunStatus::Completed
                } else {
                    ChatRunStatus::Failed
                };
                let assistant_content = if !ar.response.trim().is_empty() {
                    ar.response.clone()
                } else if !ar.error.trim().is_empty() {
                    format!("Error: {error}", error = ar.error)
                } else {
                    String::new()
                };
                let resolved_sidecar_session_id = authoritative_sidecar_session_id
                    .clone()
                    .or_else(|| (!ar.session_id.trim().is_empty()).then(|| ar.session_id.clone()));

                if let Some(sidecar_session_id) = resolved_sidecar_session_id.clone() {
                    let _ = chat_state::set_session_sidecar_session_id(
                        &session_id,
                        Some(sidecar_session_id),
                    );
                }

                let _ = chat_state::update_run(&run_id, |run| {
                    run.status = final_status.clone();
                    run.completed_at = Some(completed_at);
                    if let Some(sidecar_session_id) = resolved_sidecar_session_id.clone() {
                        run.sidecar_session_id = Some(sidecar_session_id);
                    }
                    if !ar.trace_id.trim().is_empty() {
                        run.trace_id = Some(ar.trace_id.clone());
                    }
                    if !ar.response.trim().is_empty() {
                        run.final_output = Some(ar.response.clone());
                    }
                    if !ar.error.trim().is_empty() {
                        run.error = Some(ar.error.clone());
                    }
                });
                let _ = chat_state::clear_session_active_run(&session_id);

                let mut assistant_message = get_or_create_assistant_message(
                    &session_id,
                    &assistant_message_id,
                    &run_id,
                    assistant_started_at,
                );
                if assistant_message.parts.is_empty() && !assistant_content.is_empty() {
                    assistant_message.parts.push(json!({
                        "id": format!("text-{}", uuid::Uuid::new_v4()),
                        "type": "text",
                        "text": assistant_content.clone(),
                    }));
                }
                finalize_streamed_assistant_parts(&mut assistant_message.parts, completed_at);
                assistant_message.content = assistant_content;
                assistant_message.completed_at = Some(completed_at);
                assistant_message.trace_id = if ar.trace_id.trim().is_empty() {
                    None
                } else {
                    Some(ar.trace_id.clone())
                };
                assistant_message.success = Some(ar.success);
                assistant_message.error = if ar.error.trim().is_empty() {
                    None
                } else {
                    Some(ar.error.clone())
                };
                let _ = chat_state::append_message(&session_id, assistant_message.clone());
                emit_message_updated(&session_id, &assistant_message);
                for part in &assistant_message.parts {
                    emit_message_part_updated(&session_id, &assistant_message.id, part.clone());
                }
                emit_session_idle(&session_id);

                if let Ok(Some(updated_run)) = chat_state::get_run(&run_id) {
                    publish_run_event(
                        &session_id,
                        if updated_run.status == ChatRunStatus::Completed {
                            "run_completed"
                        } else {
                            "run_failed"
                        },
                        &updated_run,
                    );
                    publish_run_progress(
                        &session_id,
                        &updated_run.id,
                        &updated_run.status,
                        if updated_run.status == ChatRunStatus::Completed {
                            "completed"
                        } else {
                            "failed"
                        },
                        if updated_run.status == ChatRunStatus::Completed {
                            "Run completed successfully."
                        } else {
                            "Run finished with an error."
                        },
                    );
                }
            }
            Err((status, api_error_body)) => {
                let completed_at = chat_state::now_ms();
                let error_text = api_error_body.0.error.clone();
                let _ = chat_state::update_run(&run_id, |run| {
                    run.status = ChatRunStatus::Failed;
                    run.completed_at = Some(completed_at);
                    run.error = Some(error_text.clone());
                });
                let _ = chat_state::clear_session_active_run(&session_id);

                let mut assistant_message = get_or_create_assistant_message(
                    &session_id,
                    &assistant_message_id,
                    &run_id,
                    assistant_started_at,
                );
                if !assistant_message_has_visible_text(&assistant_message.parts) {
                    assistant_message.parts.push(json!({
                        "id": format!("text-{}", uuid::Uuid::new_v4()),
                        "type": "text",
                        "text": format!("Error: {error_text}"),
                    }));
                }
                finalize_streamed_assistant_parts(&mut assistant_message.parts, completed_at);
                if assistant_message.content.trim().is_empty() {
                    assistant_message.content = format!("Error: {error_text}");
                }
                assistant_message.completed_at = Some(completed_at);
                assistant_message.trace_id = None;
                assistant_message.success = Some(false);
                assistant_message.error = Some(error_text.clone());
                let _ = chat_state::append_message(&session_id, assistant_message.clone());
                emit_message_updated(&session_id, &assistant_message);
                for part in &assistant_message.parts {
                    emit_message_part_updated(&session_id, &assistant_message.id, part.clone());
                }
                emit_session_error(&session_id, &error_text, api_error_body.0.code.as_deref());
                emit_session_idle(&session_id);

                if let Ok(Some(updated_run)) = chat_state::get_run(&run_id) {
                    publish_run_event(&session_id, "run_failed", &updated_run);
                    publish_run_progress(
                        &session_id,
                        &updated_run.id,
                        &updated_run.status,
                        "failed",
                        "Run failed before the operator received a successful result.",
                    );
                } else {
                    let _ = status;
                }
            }
        }
    });
    register_chat_run_abort(&spawned_run_id, handle.abort_handle());
}
