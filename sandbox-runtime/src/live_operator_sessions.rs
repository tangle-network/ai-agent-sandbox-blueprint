//! Reusable in-memory live terminal/chat session primitives for operator APIs.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// Default keep-alive interval for SSE streams.
const SSE_KEEP_ALIVE_SECS: u64 = 15;

/// Live terminal session metadata plus output channel.
#[derive(Clone, Debug)]
pub struct LiveTerminalSession {
    pub id: String,
    pub scope_id: String,
    pub owner: String,
    pub output_tx: broadcast::Sender<String>,
}

impl LiveTerminalSession {
    /// Create a new terminal session with a bounded output ring buffer.
    pub fn new(
        scope_id: impl Into<String>,
        owner: impl Into<String>,
        output_buffer: usize,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let (output_tx, _rx) = broadcast::channel(output_buffer);
        Self {
            id,
            scope_id: scope_id.into(),
            owner: owner.into(),
            output_tx,
        }
    }
}

/// Generic JSON SSE event.
#[derive(Clone, Debug)]
pub struct LiveJsonEvent {
    pub event_type: String,
    pub payload: Value,
}

/// Live chat session with typed message storage and JSON event stream.
#[derive(Clone, Debug)]
pub struct LiveChatSession<M> {
    pub id: String,
    pub scope_id: String,
    pub owner: String,
    pub title: String,
    pub messages: Vec<M>,
    pub events_tx: broadcast::Sender<LiveJsonEvent>,
}

impl<M> LiveChatSession<M> {
    /// Create a new empty chat session.
    pub fn new(
        scope_id: impl Into<String>,
        owner: impl Into<String>,
        title: impl Into<String>,
        events_buffer: usize,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let (events_tx, _rx) = broadcast::channel(events_buffer);
        Self {
            id,
            scope_id: scope_id.into(),
            owner: owner.into(),
            title: title.into(),
            messages: Vec::new(),
            events_tx,
        }
    }
}

/// In-memory store for terminal and chat sessions.
#[derive(Debug)]
pub struct LiveSessionStore<M> {
    terminals: Mutex<BTreeMap<String, LiveTerminalSession>>,
    chats: Mutex<BTreeMap<String, LiveChatSession<M>>>,
}

impl<M> Default for LiveSessionStore<M> {
    fn default() -> Self {
        Self {
            terminals: Mutex::new(BTreeMap::new()),
            chats: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<M> LiveSessionStore<M>
where
    M: Clone,
{
    pub fn insert_terminal(&self, session: LiveTerminalSession) -> Result<(), String> {
        self.terminals
            .lock()
            .map_err(|e| format!("terminal session lock poisoned: {e}"))?
            .insert(session.id.clone(), session);
        Ok(())
    }

    pub fn get_terminal(&self, session_id: &str) -> Result<Option<LiveTerminalSession>, String> {
        Ok(self
            .terminals
            .lock()
            .map_err(|e| format!("terminal session lock poisoned: {e}"))?
            .get(session_id)
            .cloned())
    }

    pub fn remove_terminal(&self, session_id: &str) -> Result<Option<LiveTerminalSession>, String> {
        Ok(self
            .terminals
            .lock()
            .map_err(|e| format!("terminal session lock poisoned: {e}"))?
            .remove(session_id))
    }

    pub fn list_terminals(&self) -> Result<Vec<LiveTerminalSession>, String> {
        Ok(self
            .terminals
            .lock()
            .map_err(|e| format!("terminal session lock poisoned: {e}"))?
            .values()
            .cloned()
            .collect())
    }

    pub fn insert_chat(&self, session: LiveChatSession<M>) -> Result<(), String> {
        self.chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?
            .insert(session.id.clone(), session);
        Ok(())
    }

    pub fn get_chat(&self, session_id: &str) -> Result<Option<LiveChatSession<M>>, String> {
        Ok(self
            .chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?
            .get(session_id)
            .cloned())
    }

    pub fn remove_chat(&self, session_id: &str) -> Result<Option<LiveChatSession<M>>, String> {
        Ok(self
            .chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?
            .remove(session_id))
    }

    pub fn list_chats(&self) -> Result<Vec<LiveChatSession<M>>, String> {
        Ok(self
            .chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?
            .values()
            .cloned()
            .collect())
    }

    pub fn update_chat<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut LiveChatSession<M>) -> R,
    ) -> Result<Option<R>, String> {
        let mut chats = self
            .chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?;
        let Some(chat) = chats.get_mut(session_id) else {
            return Ok(None);
        };
        Ok(Some(f(chat)))
    }

    #[cfg(test)]
    pub fn clear_all_for_testing(&self) -> Result<(), String> {
        self.terminals
            .lock()
            .map_err(|e| format!("terminal session lock poisoned: {e}"))?
            .clear();
        self.chats
            .lock()
            .map_err(|e| format!("chat session lock poisoned: {e}"))?
            .clear();
        Ok(())
    }
}

/// Convert a terminal broadcast receiver into an SSE response.
pub fn sse_from_terminal_output(
    rx: broadcast::Receiver<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(chunk) => Some(Ok(Event::default().data(chunk))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS))
            .text("keep-alive"),
    )
}

/// Convert a JSON event broadcast receiver into an SSE response.
pub fn sse_from_json_events(
    rx: broadcast::Receiver<LiveJsonEvent>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => Some(Ok(Event::default()
            .event(event.event_type)
            .data(event.payload.to_string()))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(SSE_KEEP_ALIVE_SECS))
            .text("keep-alive"),
    )
}
