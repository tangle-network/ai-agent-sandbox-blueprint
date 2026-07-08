//! Extracted from operator_api.rs — sse route group.

use super::*;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct CreateLiveChatSessionRequest {
    #[serde(default)]
    pub(crate) title: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct LiveSessionSummary {
    pub(crate) session_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_run_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LiveChatSessionDetail {
    pub(crate) session_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sidecar_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_run_id: Option<String>,
    pub(crate) messages: Vec<ChatMessageRecord>,
    pub(crate) run_progress: Vec<ChatRunProgressRecord>,
    pub(crate) runs: Vec<ChatRunRecord>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CancelChatRunResponse {
    pub(crate) success: bool,
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) status: String,
    pub(crate) cancelled_at: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalSessionDescriptor {
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) stream_path: Option<String>,
}

pub(crate) fn live_scope_sandbox(sandbox_id: &str) -> String {
    format!("sandbox:{sandbox_id}")
}

pub(crate) fn live_scope_instance(record: &SandboxRecord) -> String {
    format!("instance:{}", record.id)
}

pub(crate) fn chat_session_matches(session: &ChatSessionRecord, scope: &str, owner: &str) -> bool {
    chat_state::session_matches(session, scope, owner)
}

pub(crate) fn publish_chat_message(session_id: &str, message: ChatMessageRecord, event_type: &str) {
    if message.content.trim().is_empty() && message.role.eq_ignore_ascii_case("assistant") {
        return;
    }
    let _ = chat_state::append_message(session_id, message.clone());
    let _ = chat_state::emit_event(
        session_id,
        event_type,
        chat_state::message_event_payload(&message),
    );
}

pub(crate) fn publish_run_event(session_id: &str, event_type: &str, run: &ChatRunRecord) {
    let _ = chat_state::emit_event(session_id, event_type, chat_state::run_event_payload(run));
}

pub(crate) fn publish_run_progress(
    session_id: &str,
    run_id: &str,
    status: &ChatRunStatus,
    phase: &str,
    message: &str,
) {
    let Ok(Some(progress)) =
        chat_state::append_run_progress(session_id, run_id, status.clone(), phase, message)
    else {
        return;
    };
    let _ = chat_state::emit_event(session_id, "run_progress", json!(progress));
}

#[derive(Debug, Default)]
pub(crate) struct AgentStreamOutcome {
    pub(crate) success: bool,
    pub(crate) response: String,
    pub(crate) error: String,
    pub(crate) trace_id: String,
    pub(crate) session_id: String,
    pub(crate) duration_ms: u64,
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LiveChatSidecarSessionSource {
    None,
    Request,
    SessionUpdated,
    ExecutionStarted,
}

#[derive(Debug)]
pub(crate) struct SidecarSseEvent {
    pub(crate) event_type: String,
    pub(crate) data: Value,
}

pub(crate) fn chat_message_info_payload(session_id: &str, message: &ChatMessageRecord) -> Value {
    json!({
        "info": {
            "id": message.id,
            "role": message.role,
            "sessionID": session_id,
            "runID": message.run_id,
            "success": message.success,
            "error": message.error,
            "timestamp": message.created_at,
            "time": {
                "created": message.created_at,
                "completed": message.completed_at,
            }
        }
    })
}

pub(crate) fn emit_message_updated(session_id: &str, message: &ChatMessageRecord) {
    let _ = chat_state::emit_event(
        session_id,
        "message.updated",
        chat_message_info_payload(session_id, message),
    );
}

pub(crate) fn emit_message_part_updated(session_id: &str, message_id: &str, part: Value) {
    let mut part_object = match part {
        Value::Object(map) => map,
        _ => return,
    };
    part_object.insert("sessionID".into(), json!(session_id));
    part_object.insert("messageID".into(), json!(message_id));
    let _ = chat_state::emit_event(
        session_id,
        "message.part.updated",
        json!({ "part": Value::Object(part_object) }),
    );
}

pub(crate) fn emit_session_idle(session_id: &str) {
    let _ = chat_state::emit_event(
        session_id,
        "session.idle",
        json!({ "sessionID": session_id }),
    );
}

pub(crate) fn emit_session_error(session_id: &str, message: &str, code: Option<&str>) {
    let _ = chat_state::emit_event(
        session_id,
        "session.error",
        json!({
            "sessionID": session_id,
            "error": {
                "message": message,
                "code": code,
            }
        }),
    );
}

pub(crate) fn parse_sse_event(frame: &str) -> Option<SidecarSseEvent> {
    let mut event_type = "message".to_string();
    let mut data_lines = Vec::new();

    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let raw = data_lines.join("\n");
    let data = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!(raw));
    Some(SidecarSseEvent { event_type, data })
}

pub(crate) fn normalize_stream_part(part: &Value) -> Option<Value> {
    let mut object = part.as_object()?.clone();
    if object.get("type").and_then(Value::as_str) == Some("image") {
        return None;
    }

    if object.get("type").and_then(Value::as_str) == Some("tool")
        && let Some(state) = object.get_mut("state").and_then(Value::as_object_mut)
        && state.get("status").and_then(Value::as_str) == Some("failed")
    {
        state.insert("status".into(), json!("error"));
    }

    Some(Value::Object(object))
}

pub(crate) fn should_forward_stream_part(
    part: &Value,
    request_text: &str,
    ignored_upstream_message_ids: &mut HashSet<String>,
    assistant_upstream_message_ids: &mut HashSet<String>,
) -> bool {
    let request_text = request_text.trim();
    let is_exact_request_echo = !request_text.is_empty()
        && part.get("type").and_then(Value::as_str) == Some("text")
        && part.get("text").and_then(Value::as_str).map(str::trim) == Some(request_text);

    let upstream_message_id = part
        .get("messageID")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());

    if is_exact_request_echo {
        if let Some(upstream_message_id) = upstream_message_id {
            ignored_upstream_message_ids.insert(upstream_message_id.to_string());
        }
        return false;
    }

    let Some(upstream_message_id) = upstream_message_id else {
        return true;
    };

    if ignored_upstream_message_ids.contains(upstream_message_id) {
        return false;
    }
    if assistant_upstream_message_ids.contains(upstream_message_id) {
        return true;
    }

    assistant_upstream_message_ids.insert(upstream_message_id.to_string());
    true
}

pub(crate) fn finalize_streamed_assistant_parts(parts: &mut [Value], completed_at: u64) {
    for part in parts {
        let Some(object) = part.as_object_mut() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }

        let Some(time) = object.get_mut("time").and_then(Value::as_object_mut) else {
            continue;
        };
        if time.get("start").is_some() && time.get("end").is_none() {
            time.insert("end".into(), json!(completed_at));
        }
    }
}

pub(crate) fn assistant_message_has_visible_text(parts: &[Value]) -> bool {
    parts.iter().any(|part| {
        let Some(object) = part.as_object() else {
            return false;
        };
        object.get("type").and_then(Value::as_str) == Some("text")
            && object
                .get("text")
                .and_then(Value::as_str)
                .map(|text| !text.trim().is_empty())
                .unwrap_or(false)
    })
}

pub(crate) fn get_or_create_assistant_message(
    session_id: &str,
    assistant_message_id: &str,
    run_id: &str,
    assistant_started_at: u64,
) -> ChatMessageRecord {
    chat_state::get_session(session_id)
        .ok()
        .flatten()
        .and_then(|session| {
            session
                .messages
                .into_iter()
                .find(|entry| entry.id == assistant_message_id)
        })
        .unwrap_or(ChatMessageRecord {
            id: assistant_message_id.to_string(),
            run_id: Some(run_id.to_string()),
            role: "assistant".to_string(),
            content: String::new(),
            created_at: assistant_started_at,
            completed_at: None,
            parts: Vec::new(),
            trace_id: None,
            success: None,
            error: None,
        })
}

pub(crate) fn parse_agent_stream_result(parsed: &Value) -> AgentStreamOutcome {
    let final_text = parsed
        .get("finalText")
        .or_else(|| parsed.get("response"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let metadata = parsed.get("metadata");
    let session_id = metadata
        .and_then(|meta| meta.get("sessionId"))
        .or_else(|| parsed.get("sessionId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let trace_id = metadata
        .and_then(|meta| meta.get("traceId"))
        .or_else(|| parsed.get("traceId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let token_usage = parsed.get("tokenUsage").or_else(|| parsed.get("usage"));
    let timing = parsed.get("timing");

    AgentStreamOutcome {
        success: true,
        response: final_text,
        error: String::new(),
        trace_id,
        session_id,
        duration_ms: timing
            .and_then(|value| value.get("totalMs").or_else(|| value.get("duration_ms")))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        input_tokens: token_usage
            .and_then(|value| {
                value
                    .get("inputTokens")
                    .or_else(|| value.get("input_tokens"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: token_usage
            .and_then(|value| {
                value
                    .get("outputTokens")
                    .or_else(|| value.get("output_tokens"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    }
}

pub(crate) fn extract_stream_session_id(data: &Value) -> Option<String> {
    data.get("sessionId")
        .or_else(|| data.get("sessionID"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn register_chat_run_abort(run_id: &str, abort_handle: AbortHandle) {
    if let Ok(mut handles) = CHAT_RUN_ABORTS.lock() {
        handles.insert(run_id.to_string(), abort_handle);
    }
}

pub(crate) fn clear_chat_run_abort(run_id: &str) {
    if let Ok(mut handles) = CHAT_RUN_ABORTS.lock() {
        handles.remove(run_id);
    }
}

pub(crate) fn abort_chat_run_task(run_id: &str) -> bool {
    match CHAT_RUN_ABORTS.lock() {
        Ok(mut handles) => {
            if let Some(handle) = handles.remove(run_id) {
                handle.abort();
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}
