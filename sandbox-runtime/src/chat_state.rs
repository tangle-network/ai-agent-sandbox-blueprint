use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::live_operator_sessions::LiveJsonEvent;
use crate::store::{self, PersistentStore};

const CHAT_EVENT_BUFFER: usize = 256;
const CHAT_RUN_PROGRESS_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunKind {
    Prompt,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl ChatRunStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionRecord {
    pub id: String,
    pub scope_id: String,
    pub owner: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_sidecar_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    #[serde(default)]
    pub run_progress: Vec<ChatRunProgressRecord>,
    #[serde(default = "default_next_progress_seq")]
    pub next_progress_seq: u64,
    pub messages: Vec<ChatMessageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRunRecord {
    pub id: String,
    pub session_id: String,
    pub scope_id: String,
    pub owner: String,
    pub kind: ChatRunKind,
    pub status: ChatRunStatus,
    pub request_text: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRunProgressRecord {
    pub seq: u64,
    pub run_id: String,
    pub status: ChatRunStatus,
    pub phase: String,
    pub message: String,
    pub timestamp_ms: u64,
}

static CHAT_SESSIONS: OnceCell<PersistentStore<ChatSessionRecord>> = OnceCell::new();
static CHAT_RUNS: OnceCell<PersistentStore<ChatRunRecord>> = OnceCell::new();
static CHAT_STREAMS: Lazy<Mutex<HashMap<String, broadcast::Sender<LiveJsonEvent>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static CHAT_INIT: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

fn default_next_progress_seq() -> u64 {
    1
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn session_store_raw() -> Result<&'static PersistentStore<ChatSessionRecord>, String> {
    CHAT_SESSIONS
        .get_or_try_init(|| {
            let path = store::state_dir().join("chat-sessions.json");
            PersistentStore::open(path).map_err(|e| e.to_string())
        })
        .map_err(|e| e.clone())
}

fn run_store_raw() -> Result<&'static PersistentStore<ChatRunRecord>, String> {
    CHAT_RUNS
        .get_or_try_init(|| {
            let path = store::state_dir().join("chat-runs.json");
            PersistentStore::open(path).map_err(|e| e.to_string())
        })
        .map_err(|e| e.clone())
}

fn ensure_initialized() -> Result<(), String> {
    let mut initialized = CHAT_INIT
        .lock()
        .map_err(|e| format!("chat init lock poisoned: {e}"))?;
    if *initialized {
        return Ok(());
    }

    let runs = run_store_raw()?;
    let sessions = session_store_raw()?;
    let mut interrupted_session_ids = Vec::new();
    let all_runs = runs.values().map_err(|e| e.to_string())?;
    let interrupted_at = now_ms();

    for run in all_runs {
        if !run.status.is_active() {
            continue;
        }
        interrupted_session_ids.push(run.session_id.clone());
        runs.update(&run.id, |entry| {
            entry.status = ChatRunStatus::Interrupted;
            entry.error = Some("Operator restarted before the run completed".to_string());
            entry.completed_at = Some(interrupted_at);
        })
        .map_err(|e| e.to_string())?;
    }

    for session_id in interrupted_session_ids {
        let _ = sessions
            .update(&session_id, |session| {
                session.active_run_id = None;
                session.updated_at = interrupted_at;
            })
            .map_err(|e| e.to_string())?;
    }

    *initialized = true;
    Ok(())
}

pub fn session_store() -> Result<&'static PersistentStore<ChatSessionRecord>, String> {
    ensure_initialized()?;
    session_store_raw()
}

pub fn run_store() -> Result<&'static PersistentStore<ChatRunRecord>, String> {
    ensure_initialized()?;
    run_store_raw()
}

pub fn create_session(
    scope_id: &str,
    owner: &str,
    title: Option<&str>,
) -> Result<ChatSessionRecord, String> {
    let created_at = now_ms();
    let record = ChatSessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        scope_id: scope_id.to_string(),
        owner: owner.to_string(),
        title: title
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("New Chat")
            .to_string(),
        created_at,
        updated_at: created_at,
        latest_sidecar_session_id: None,
        active_run_id: None,
        run_progress: Vec::new(),
        next_progress_seq: default_next_progress_seq(),
        messages: Vec::new(),
    };
    session_store()?
        .insert(record.id.clone(), record.clone())
        .map_err(|e| e.to_string())?;
    Ok(record)
}

pub fn list_sessions(scope_id: &str, owner: &str) -> Result<Vec<ChatSessionRecord>, String> {
    let mut sessions = session_store()?
        .values()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|session| session.scope_id == scope_id && session.owner.eq_ignore_ascii_case(owner))
        .collect::<Vec<_>>();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    Ok(sessions)
}

pub fn get_session(session_id: &str) -> Result<Option<ChatSessionRecord>, String> {
    session_store()?.get(session_id).map_err(|e| e.to_string())
}

pub fn session_matches(session: &ChatSessionRecord, scope_id: &str, owner: &str) -> bool {
    session.scope_id == scope_id && session.owner.eq_ignore_ascii_case(owner)
}

pub fn delete_session(session_id: &str) -> Result<(), String> {
    let runs = list_runs_for_session(session_id)?;
    let run_store = run_store()?;
    for run in runs {
        run_store.remove(&run.id).map_err(|e| e.to_string())?;
    }
    session_store()?
        .remove(session_id)
        .map_err(|e| e.to_string())?;
    CHAT_STREAMS
        .lock()
        .map_err(|e| format!("chat stream lock poisoned: {e}"))?
        .remove(session_id);
    Ok(())
}

pub fn append_message(session_id: &str, message: ChatMessageRecord) -> Result<bool, String> {
    let updated = session_store()?
        .update(session_id, |session| {
            if let Some(existing) = session.messages.iter_mut().find(|entry| entry.id == message.id) {
                *existing = message.clone();
            } else {
                session.messages.push(message.clone());
            }
            session.updated_at = message.completed_at.unwrap_or(message.created_at);
        })
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

fn part_identity(part: &Value) -> Option<(Option<String>, Option<String>)> {
    let object = part.as_object()?;
    let part_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let part_id = object.get("id").and_then(Value::as_str).map(str::to_string);
    Some((part_type, part_id))
}

fn matches_part(existing: &Value, incoming: &Value) -> bool {
    let Some((incoming_type, incoming_id)) = part_identity(incoming) else {
        return false;
    };
    let Some((existing_type, existing_id)) = part_identity(existing) else {
        return false;
    };

    if incoming_id.is_some() && existing_id.is_some() {
        return incoming_id == existing_id;
    }

    incoming_type == Some("text".to_string()) && existing_type == incoming_type
}

fn visible_text_from_parts(parts: &[Value]) -> String {
    parts.iter()
        .rev()
        .find_map(|part| {
            let object = part.as_object()?;
            if object.get("type").and_then(Value::as_str) != Some("text") {
                return None;
            }
            object.get("text").and_then(Value::as_str).map(str::to_string)
        })
        .unwrap_or_default()
}

pub fn upsert_message_part(
    session_id: &str,
    message_id: &str,
    part: Value,
) -> Result<bool, String> {
    let timestamp = now_ms();
    session_store()?
        .update(session_id, |session| {
            let Some(message) = session.messages.iter_mut().find(|entry| entry.id == message_id) else {
                return;
            };

            if let Some(index) = message.parts.iter().position(|existing| matches_part(existing, &part))
            {
                message.parts[index] = part.clone();
            } else {
                message.parts.push(part.clone());
            }

            let text_content = visible_text_from_parts(&message.parts);
            if !text_content.is_empty() {
                message.content = text_content;
            }
            session.updated_at = timestamp;
        })
        .map_err(|e| e.to_string())
}

pub fn create_run(
    session_id: &str,
    scope_id: &str,
    owner: &str,
    kind: ChatRunKind,
    request_text: &str,
) -> Result<ChatRunRecord, String> {
    let created_at = now_ms();
    let run = ChatRunRecord {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        scope_id: scope_id.to_string(),
        owner: owner.to_string(),
        kind,
        status: ChatRunStatus::Queued,
        request_text: request_text.to_string(),
        created_at,
        started_at: None,
        completed_at: None,
        sidecar_session_id: None,
        trace_id: None,
        final_output: None,
        error: None,
    };
    run_store()?
        .insert(run.id.clone(), run.clone())
        .map_err(|e| e.to_string())?;
    let _ = session_store()?
        .update(session_id, |session| {
            session.active_run_id = Some(run.id.clone());
            session.updated_at = created_at;
        })
        .map_err(|e| e.to_string())?;
    Ok(run)
}

pub fn get_run(run_id: &str) -> Result<Option<ChatRunRecord>, String> {
    run_store()?.get(run_id).map_err(|e| e.to_string())
}

pub fn update_run(run_id: &str, f: impl FnOnce(&mut ChatRunRecord)) -> Result<bool, String> {
    run_store()?.update(run_id, f).map_err(|e| e.to_string())
}

pub fn list_runs_for_session(session_id: &str) -> Result<Vec<ChatRunRecord>, String> {
    let mut runs = run_store()?
        .values()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|run| run.session_id == session_id)
        .collect::<Vec<_>>();
    runs.sort_by_key(|run| run.created_at);
    Ok(runs)
}

pub fn active_run_for_scope(scope_id: &str, owner: &str) -> Result<Option<ChatRunRecord>, String> {
    let active = run_store()?
        .values()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|run| {
            run.scope_id == scope_id
                && run.owner.eq_ignore_ascii_case(owner)
                && run.status.is_active()
        });
    Ok(active)
}

pub fn set_session_sidecar_session_id(
    session_id: &str,
    sidecar_session_id: Option<String>,
) -> Result<bool, String> {
    let timestamp = now_ms();
    session_store()?
        .update(session_id, |session| {
            session.latest_sidecar_session_id = sidecar_session_id.clone();
            session.updated_at = timestamp;
        })
        .map_err(|e| e.to_string())
}

pub fn append_run_progress(
    session_id: &str,
    run_id: &str,
    status: ChatRunStatus,
    phase: &str,
    message: &str,
) -> Result<Option<ChatRunProgressRecord>, String> {
    let timestamp = now_ms();
    let mut appended = None;
    let updated = session_store()?
        .update(session_id, |session| {
            let seq = if session.next_progress_seq == 0 {
                (session.run_progress.len() as u64) + 1
            } else {
                session.next_progress_seq
            };
            let entry = ChatRunProgressRecord {
                seq,
                run_id: run_id.to_string(),
                status: status.clone(),
                phase: phase.to_string(),
                message: message.to_string(),
                timestamp_ms: timestamp,
            };
            session.run_progress.push(entry.clone());
            if session.run_progress.len() > CHAT_RUN_PROGRESS_LIMIT {
                let overflow = session.run_progress.len() - CHAT_RUN_PROGRESS_LIMIT;
                session.run_progress.drain(0..overflow);
            }
            session.next_progress_seq = seq.saturating_add(1);
            session.updated_at = timestamp;
            appended = Some(entry);
        })
        .map_err(|e| e.to_string())?;
    if updated { Ok(appended) } else { Ok(None) }
}

pub fn list_run_progress_for_session(
    session_id: &str,
) -> Result<Vec<ChatRunProgressRecord>, String> {
    Ok(get_session(session_id)?
        .map(|session| session.run_progress)
        .unwrap_or_default())
}

pub fn maybe_auto_title_session(session_id: &str, request_text: &str) -> Result<bool, String> {
    let candidate = request_text.trim();
    if candidate.is_empty() {
        return Ok(false);
    }
    let title = if candidate.len() > 40 {
        format!("{}...", &candidate[..40])
    } else {
        candidate.to_string()
    };
    let timestamp = now_ms();
    session_store()?
        .update(session_id, |session| {
            if session.messages.is_empty()
                && (session.title == "New Chat" || session.title == "Chat Session")
            {
                session.title = title.clone();
                session.updated_at = timestamp;
            }
        })
        .map_err(|e| e.to_string())
}

pub fn clear_session_active_run(session_id: &str) -> Result<bool, String> {
    let timestamp = now_ms();
    session_store()?
        .update(session_id, |session| {
            session.active_run_id = None;
            session.updated_at = timestamp;
        })
        .map_err(|e| e.to_string())
}

fn sender_for_session(session_id: &str) -> Result<broadcast::Sender<LiveJsonEvent>, String> {
    let mut streams = CHAT_STREAMS
        .lock()
        .map_err(|e| format!("chat stream lock poisoned: {e}"))?;
    Ok(streams
        .entry(session_id.to_string())
        .or_insert_with(|| {
            let (sender, _rx) = broadcast::channel(CHAT_EVENT_BUFFER);
            sender
        })
        .clone())
}

pub fn subscribe_events(session_id: &str) -> Result<broadcast::Receiver<LiveJsonEvent>, String> {
    Ok(sender_for_session(session_id)?.subscribe())
}

pub fn emit_event(session_id: &str, event_type: &str, payload: Value) -> Result<(), String> {
    let _ = sender_for_session(session_id)?.send(LiveJsonEvent {
        event_type: event_type.to_string(),
        payload,
    });
    Ok(())
}

pub fn message_event_payload(message: &ChatMessageRecord) -> Value {
    json!(message)
}

pub fn run_event_payload(run: &ChatRunRecord) -> Value {
    json!(run)
}

#[cfg(test)]
pub fn clear_all_for_testing() -> Result<(), String> {
    if let Some(store) = CHAT_SESSIONS.get() {
        store.replace(HashMap::new()).map_err(|e| e.to_string())?;
    }
    if let Some(store) = CHAT_RUNS.get() {
        store.replace(HashMap::new()).map_err(|e| e.to_string())?;
    }
    CHAT_STREAMS
        .lock()
        .map_err(|e| format!("chat stream lock poisoned: {e}"))?
        .clear();
    *CHAT_INIT
        .lock()
        .map_err(|e| format!("chat init lock poisoned: {e}"))? = false;
    Ok(())
}
