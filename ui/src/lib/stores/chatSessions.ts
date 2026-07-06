import { atom } from 'nanostores';
import type {
  ChatStreamEvent,
  ChatRunSummary,
  ChatSessionDetail,
  SandboxClient,
} from '~/lib/api/sandboxClient';
import type { ChatRunEntry, ChatSessionEntry } from './chatSessions.types';
import { applySessionDetail, applySessionSummary } from './chatSessions.mappers';
import {
  applyMessagePartUpdated,
  applyMessageUpdated,
  applyRunProgress,
  applyRunUpdate,
  applyStreamMessage,
} from './chatSessions.stream';

export type { ChatRunEntry, ChatRunProgressEntry, ChatSessionEntry } from './chatSessions.types';

interface ChatSessionsState {
  sessions: Record<string, ChatSessionEntry[]>;
  active: Record<string, string>;
  loading: Record<string, boolean>;
  error: Record<string, string | null>;
}

if (typeof window !== 'undefined') {
  try { localStorage.removeItem('chat_sessions'); } catch { /* ignore */ }
}

export const chatSessionsStore = atom<ChatSessionsState>({
  sessions: {},
  active: {},
  loading: {},
  error: {},
});

function update(fn: (state: ChatSessionsState) => ChatSessionsState) {
  chatSessionsStore.set(fn(chatSessionsStore.get()));
}

export function hasActiveRun(session: ChatSessionEntry | null | undefined): boolean {
  return !!session?.activeRunId;
}

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
  return (state.sessions[sandboxId] ?? []).find((session) => session.id === activeId);
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
  update((state) => ({
    ...state,
    sessions: {
      ...state.sessions,
      [sandboxId]: (state.sessions[sandboxId] ?? []).map((session) =>
        session.id === sessionId ? { ...session, title } : session,
      ),
    },
  }));
}

export function markRunAccepted(
  sandboxId: string,
  sessionId: string,
  runId: string,
  status: ChatRunEntry['status'],
  acceptedAt: number,
  kind: ChatRunEntry['kind'],
  requestText: string,
) {
  update((state) => ({
    ...state,
    sessions: {
      ...state.sessions,
      [sandboxId]: (state.sessions[sandboxId] ?? []).map((session) => {
        if (session.id !== sessionId) return session;
        const runs = session.runs.some((run) => run.id === runId)
          ? session.runs.map((run) => (run.id === runId ? { ...run, status } : run))
          : [
            ...session.runs,
            {
              id: runId,
              kind,
              status,
              requestText,
              createdAt: acceptedAt,
            },
          ];
        return {
          ...session,
          activeRunId: runId,
          runs,
        };
      }),
    },
  }));
}

export async function fetchSessions(client: SandboxClient, sandboxId: string): Promise<void> {
  if (chatSessionsStore.get().loading[sandboxId]) return;

  update((state) => ({
    ...state,
    loading: { ...state.loading, [sandboxId]: true },
    error: { ...state.error, [sandboxId]: null },
  }));

  try {
    const summaries = await client.listChatSessions();
    update((state) => {
      const currentSessions = state.sessions[sandboxId] ?? [];
      const entries = summaries.map((summary) =>
        applySessionSummary(
          sandboxId,
          currentSessions.find((session) => session.id === summary.session_id),
          summary,
        ),
      );
      const currentActive = state.active[sandboxId];
      const activeStillExists = entries.some((entry) => entry.id === currentActive);
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
    const message = err instanceof Error ? err.message : 'Failed to load sessions';
    update((state) => ({
      ...state,
      loading: { ...state.loading, [sandboxId]: false },
      error: { ...state.error, [sandboxId]: message },
    }));
  }
}

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
      sidecarSessionId: undefined,
      activeRunId: result.active_run_id ?? undefined,
      runs: [],
      runProgress: [],
      messages: [],
      partMap: {},
      detailLoaded: true,
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
    const message = err instanceof Error ? err.message : 'Failed to create session';
    update((state) => ({
      ...state,
      error: { ...state.error, [sandboxId]: message },
    }));
    return null;
  }
}

export async function deleteSessionApi(
  client: SandboxClient,
  sandboxId: string,
  sessionId: string,
): Promise<void> {
  const previousState = chatSessionsStore.get();
  const previousSessions = previousState.sessions[sandboxId] ?? [];

  update((state) => {
    const remaining = (state.sessions[sandboxId] ?? []).filter((session) => session.id !== sessionId);
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
    const message = err instanceof Error ? err.message : 'Failed to delete session';
    update((state) => ({
      ...state,
      sessions: { ...state.sessions, [sandboxId]: previousSessions },
      active: { ...state.active, [sandboxId]: previousState.active[sandboxId] },
      error: { ...state.error, [sandboxId]: message },
    }));
  }
}

export async function loadSessionDetail(
  client: SandboxClient,
  sandboxId: string,
  sessionId: string,
): Promise<void> {
  try {
    const detail = await client.getChatSession(sessionId);
    update((state) => {
      const sessions = (state.sessions[sandboxId] ?? []).map((session) => {
        if (session.id !== sessionId) return session;
        return applySessionDetail(session, detail);
      });
      return { ...state, sessions: { ...state.sessions, [sandboxId]: sessions } };
    });
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Failed to load session';
    update((state) => ({
      ...state,
      error: { ...state.error, [sandboxId]: message },
    }));
  }
}

export function applyChatStreamEvent(
  sandboxId: string,
  sessionId: string,
  event: ChatStreamEvent,
) {
  update((state) => ({
    ...state,
    sessions: {
      ...state.sessions,
      [sandboxId]: (state.sessions[sandboxId] ?? []).map((session) => {
        if (session.id !== sessionId) return session;

        if (event.type === 'user_message' || event.type === 'assistant_message') {
          return applyStreamMessage(
            session,
            event.data as ChatSessionDetail['messages'][number],
          );
        }

        if (event.type === 'message.updated') {
          return applyMessageUpdated(
            session,
            event.data as Record<string, unknown>,
          );
        }

        if (event.type === 'message.part.updated') {
          return applyMessagePartUpdated(
            session,
            event.data as Record<string, unknown>,
          );
        }

        if (
          event.type === 'run_queued'
          || event.type === 'run_started'
          || event.type === 'run_cancel_requested'
          || event.type === 'run_completed'
          || event.type === 'run_failed'
          || event.type === 'run_cancelled'
        ) {
          return applyRunUpdate(session, event.data as ChatRunSummary);
        }

        if (event.type === 'run_progress') {
          return applyRunProgress(
            session,
            event.data as Parameters<typeof applyRunProgress>[1],
          );
        }

        return session;
      }),
    },
  }));
}
