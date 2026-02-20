import { atom } from 'nanostores';
import type { SessionMessage, SessionPart } from '@tangle/agent-ui';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ChatSessionEntry {
  id: string;
  /** Session ID returned by sidecar for conversation continuity */
  sidecarSessionId?: string;
  title: string;
  sandboxId: string;
  createdAt: number;
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
}

interface ChatSessionsState {
  /** All sessions keyed by sandboxId → ChatSessionEntry[] */
  sessions: Record<string, ChatSessionEntry[]>;
  /** Active session ID per sandbox */
  active: Record<string, string>;
}

// ---------------------------------------------------------------------------
// localStorage persistence
// ---------------------------------------------------------------------------

const STORAGE_KEY = 'chat_sessions';

function load(): ChatSessionsState {
  if (typeof window === 'undefined') return { sessions: {}, active: {} };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as ChatSessionsState;
  } catch { /* corrupt data */ }
  return { sessions: {}, active: {} };
}

function persist(state: ChatSessionsState) {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch { /* storage full */ }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export const chatSessionsStore = atom<ChatSessionsState>(load());

// Auto-persist on changes
chatSessionsStore.subscribe((state) => persist(state));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function genId(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

function update(fn: (state: ChatSessionsState) => ChatSessionsState) {
  chatSessionsStore.set(fn(chatSessionsStore.get()));
}

// ---------------------------------------------------------------------------
// API
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

export function setActiveSession(sandboxId: string, sessionId: string) {
  update((state) => ({
    ...state,
    active: { ...state.active, [sandboxId]: sessionId },
  }));
}

export function createSession(sandboxId: string, title?: string): ChatSessionEntry {
  const entry: ChatSessionEntry = {
    id: genId(),
    title: title ?? 'New Chat',
    sandboxId,
    createdAt: Date.now(),
    messages: [],
    partMap: {},
  };

  update((state) => {
    const existing = state.sessions[sandboxId] ?? [];
    return {
      sessions: { ...state.sessions, [sandboxId]: [entry, ...existing] },
      active: { ...state.active, [sandboxId]: entry.id },
    };
  });

  return entry;
}

export function deleteSession(sandboxId: string, sessionId: string) {
  update((state) => {
    const existing = (state.sessions[sandboxId] ?? []).filter((s) => s.id !== sessionId);
    const active = state.active[sandboxId] === sessionId
      ? existing[0]?.id
      : state.active[sandboxId];
    return {
      sessions: { ...state.sessions, [sandboxId]: existing },
      active: { ...state.active, [sandboxId]: active ?? '' },
    };
  });
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
