import { atom } from 'nanostores';
import type { SessionMessage, SessionPart, TextPart } from '@tangle-network/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ChatSessionEntry {
  /** Server-assigned session ID (canonical) */
  id: string;
  /** Session ID returned by sidecar for conversation continuity */
  sidecarSessionId?: string;
  title: string;
  sandboxId: string;
  createdAt: number;
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
  /** Whether messages have been fetched from the server */
  messagesLoaded: boolean;
}

interface ChatSessionsState {
  /** All sessions keyed by sandboxId → ChatSessionEntry[] */
  sessions: Record<string, ChatSessionEntry[]>;
  /** Active session ID per sandbox */
  active: Record<string, string>;
  /** Loading state per sandbox */
  loading: Record<string, boolean>;
  /** Error state per sandbox */
  error: Record<string, string | null>;
}

// ---------------------------------------------------------------------------
// Clean up stale localStorage data from previous implementation
// ---------------------------------------------------------------------------

if (typeof window !== 'undefined') {
  try { localStorage.removeItem('chat_sessions'); } catch { /* ignore */ }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export const chatSessionsStore = atom<ChatSessionsState>({
  sessions: {},
  active: {},
  loading: {},
  error: {},
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function update(fn: (state: ChatSessionsState) => ChatSessionsState) {
  chatSessionsStore.set(fn(chatSessionsStore.get()));
}

/** Convert a server message into the local SessionMessage + SessionPart[] format. */
function serverMessageToLocal(
  msg: { role: string; content: string },
  index: number,
): { message: SessionMessage; parts: SessionPart[] } {
  const part: TextPart = { type: 'text', text: msg.content ?? '' };
  return {
    message: {
      id: `server-${index}`,
      role: msg.role as 'user' | 'assistant',
      time: { created: Date.now() },
    },
    parts: [part],
  };
}

// ---------------------------------------------------------------------------
// Synchronous local-state API (unchanged interface)
// ---------------------------------------------------------------------------

export function getSessions(sandboxId: string): ChatSessionEntry[] {
  return chatSessionsStore.get().sessions[sandboxId] ?? [];
}

export function getActiveSessionId(sandboxId: string): string | undefined {
  return chatSessionsStore.get().active[sandboxId];
}

export function getActiveSession(sandboxId: string): ChatSessionEntry | undefined {
  const state = chatSessionsStore.get();
  const activeId = state.active[sandboxId];
  if (!activeId) return undefined;
  return (state.sessions[sandboxId] ?? []).find((s) => s.id === activeId);
}

export function getLoading(sandboxId: string): boolean {
  return chatSessionsStore.get().loading[sandboxId] ?? false;
}

export function getError(sandboxId: string): string | null {
  return chatSessionsStore.get().error[sandboxId] ?? null;
}

export function setActiveSession(sandboxId: string, sessionId: string) {
  update((state) => ({
    ...state,
    active: { ...state.active, [sandboxId]: sessionId },
  }));
}

export function renameSession(sandboxId: string, sessionId: string, title: string) {
  update((state) => {
    const sessions = (state.sessions[sandboxId] ?? []).map((s) =>
      s.id === sessionId ? { ...s, title } : s,
    );
    return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
  });
}

export function updateSessionSidecarId(sandboxId: string, sessionId: string, sidecarSessionId: string) {
  update((state) => {
    const sessions = (state.sessions[sandboxId] ?? []).map((s) =>
      s.id === sessionId ? { ...s, sidecarSessionId } : s,
    );
    return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
  });
}

export function appendMessage(sandboxId: string, sessionId: string, message: SessionMessage, parts: SessionPart[]) {
  update((state) => {
    const sessions = (state.sessions[sandboxId] ?? []).map((s) => {
      if (s.id !== sessionId) return s;
      return {
        ...s,
        messages: [...s.messages, message],
        partMap: { ...s.partMap, [message.id]: parts },
      };
    });
    return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
  });
}

export function updateParts(sandboxId: string, sessionId: string, messageId: string, parts: SessionPart[]) {
  update((state) => {
    const sessions = (state.sessions[sandboxId] ?? []).map((s) => {
      if (s.id !== sessionId) return s;
      return {
        ...s,
        partMap: { ...s.partMap, [messageId]: parts },
      };
    });
    return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
  });
}

// ---------------------------------------------------------------------------
// Async API-backed functions
// ---------------------------------------------------------------------------

/** Fetch sessions from the server and populate the store. */
export async function fetchSessions(client: SandboxClient, sandboxId: string): Promise<void> {
  // Skip if already loading
  if (chatSessionsStore.get().loading[sandboxId]) return;

  update((state) => ({
    ...state,
    loading: { ...state.loading, [sandboxId]: true },
    error: { ...state.error, [sandboxId]: null },
  }));

  try {
    const summaries = await client.listChatSessions();
    const entries: ChatSessionEntry[] = summaries.map((s) => ({
      id: s.session_id,
      title: s.title,
      sandboxId,
      createdAt: Date.now(),
      messages: [],
      partMap: {},
      messagesLoaded: false,
    }));

    update((state) => {
      const currentActive = state.active[sandboxId];
      const activeStillExists = entries.some((e) => e.id === currentActive);
      return {
        ...state,
        sessions: { ...state.sessions, [sandboxId]: entries },
        active: {
          ...state.active,
          [sandboxId]: activeStillExists ? currentActive : (entries[0]?.id ?? ''),
        },
        loading: { ...state.loading, [sandboxId]: false },
      };
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : 'Failed to load sessions';
    update((state) => ({
      ...state,
      loading: { ...state.loading, [sandboxId]: false },
      error: { ...state.error, [sandboxId]: msg },
    }));
  }
}

/** Create a new session via the API and add it to the store. */
export async function createSessionApi(
  client: SandboxClient,
  sandboxId: string,
  title?: string,
): Promise<ChatSessionEntry | null> {
  try {
    const result = await client.createChatSession(title ?? 'New Chat');
    const entry: ChatSessionEntry = {
      id: result.session_id,
      title: result.title,
      sandboxId,
      createdAt: Date.now(),
      messages: [],
      partMap: {},
      messagesLoaded: true, // new session has no messages
    };

    update((state) => {
      const existing = state.sessions[sandboxId] ?? [];
      return {
        ...state,
        sessions: { ...state.sessions, [sandboxId]: [entry, ...existing] },
        active: { ...state.active, [sandboxId]: entry.id },
        error: { ...state.error, [sandboxId]: null },
      };
    });

    return entry;
  } catch (err) {
    const msg = err instanceof Error ? err.message : 'Failed to create session';
    update((state) => ({
      ...state,
      error: { ...state.error, [sandboxId]: msg },
    }));
    return null;
  }
}

/** Delete a session via the API and remove it from the store. */
export async function deleteSessionApi(
  client: SandboxClient,
  sandboxId: string,
  sessionId: string,
): Promise<void> {
  // Optimistically remove from store
  const prevState = chatSessionsStore.get();
  const prevSessions = prevState.sessions[sandboxId] ?? [];

  update((state) => {
    const remaining = (state.sessions[sandboxId] ?? []).filter((s) => s.id !== sessionId);
    const active = state.active[sandboxId] === sessionId
      ? remaining[0]?.id ?? ''
      : state.active[sandboxId];
    return {
      ...state,
      sessions: { ...state.sessions, [sandboxId]: remaining },
      active: { ...state.active, [sandboxId]: active },
    };
  });

  try {
    await client.deleteChatSession(sessionId);
  } catch (err) {
    // Restore on failure
    const msg = err instanceof Error ? err.message : 'Failed to delete session';
    update((state) => ({
      ...state,
      sessions: { ...state.sessions, [sandboxId]: prevSessions },
      active: { ...state.active, [sandboxId]: prevState.active[sandboxId] },
      error: { ...state.error, [sandboxId]: msg },
    }));
  }
}

/** Load messages for a session from the server. */
export async function loadSessionMessages(
  client: SandboxClient,
  sandboxId: string,
  sessionId: string,
): Promise<void> {
  try {
    const detail = await client.getChatSession(sessionId);
    const converted = (detail.messages ?? []).map((m, i) =>
      serverMessageToLocal(m as { role: string; content: string }, i),
    );

    const messages = converted.map((c) => c.message);
    const partMap: Record<string, SessionPart[]> = {};
    for (const c of converted) {
      partMap[c.message.id] = c.parts;
    }

    update((state) => {
      const sessions = (state.sessions[sandboxId] ?? []).map((s) => {
        if (s.id !== sessionId) return s;
        return { ...s, messages, partMap, messagesLoaded: true };
      });
      return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : 'Failed to load messages';
    update((state) => ({
      ...state,
      error: { ...state.error, [sandboxId]: msg },
    }));
  }
}
