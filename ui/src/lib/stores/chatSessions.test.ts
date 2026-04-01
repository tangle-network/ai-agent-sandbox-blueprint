import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  applyChatStreamEvent,
  chatSessionsStore,
  createSessionApi,
  deleteSessionApi,
  fetchSessions,
  getActiveSession,
  getActiveSessionId,
  getError,
  getLoading,
  getSessions,
  hasActiveRun,
  loadSessionDetail,
  markRunAccepted,
  renameSession,
  setActiveSession,
} from './chatSessions';
import type { SandboxClient } from '~/lib/api/sandboxClient';

function resetStore() {
  chatSessionsStore.set({ sessions: {}, active: {}, loading: {}, error: {} });
}

function seedSession(sandboxId: string, id: string, title = 'Test', detailLoaded = true) {
  const state = chatSessionsStore.get();
  const entry = {
    id,
    title,
    sandboxId,
    createdAt: Date.now(),
    sidecarSessionId: undefined,
    activeRunId: undefined,
    runs: [],
    runProgress: [],
    messages: [],
    partMap: {},
    detailLoaded,
  };
  const existing = state.sessions[sandboxId] ?? [];
  chatSessionsStore.set({
    ...state,
    sessions: { ...state.sessions, [sandboxId]: [entry, ...existing] },
    active: { ...state.active, [sandboxId]: state.active[sandboxId] ?? id },
  });
  return entry;
}

function mockClient(overrides: Partial<SandboxClient> = {}): SandboxClient {
  return {
    listChatSessions: vi.fn().mockResolvedValue([]),
    createChatSession: vi.fn().mockResolvedValue({ session_id: 'new-id', title: 'New Chat' }),
    getChatSession: vi.fn().mockResolvedValue({
      session_id: 's1',
      title: 'T',
      messages: [],
      runs: [],
    }),
    deleteChatSession: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as SandboxClient;
}

beforeEach(() => {
  resetStore();
});

describe('renameSession', () => {
  it('updates the title', () => {
    seedSession('sb-1', 's1', 'Original');
    renameSession('sb-1', 's1', 'Renamed');
    expect(getSessions('sb-1')[0].title).toBe('Renamed');
  });
});

describe('markRunAccepted', () => {
  it('tracks the active run on the session', () => {
    seedSession('sb-1', 's1');
    markRunAccepted('sb-1', 's1', 'run-1', 'queued', 100, 'prompt', 'hello');
    const session = getSessions('sb-1')[0];
    expect(session.activeRunId).toBe('run-1');
    expect(session.runs[0]).toMatchObject({
      id: 'run-1',
      status: 'queued',
      kind: 'prompt',
      requestText: 'hello',
    });
    expect(hasActiveRun(session)).toBe(true);
  });
});

describe('setActiveSession / getters', () => {
  it('switches the active session', () => {
    seedSession('sb-1', 's1', 'First');
    seedSession('sb-1', 's2', 'Second');
    setActiveSession('sb-1', 's1');
    expect(getActiveSessionId('sb-1')).toBe('s1');
    expect(getActiveSession('sb-1')?.title).toBe('First');
  });
});

describe('fetchSessions', () => {
  it('populates store from API response', async () => {
    const client = mockClient({
      listChatSessions: vi.fn().mockResolvedValue([
        { session_id: 'a1', title: 'Alpha', active_run_id: 'run-a' },
        { session_id: 'b2', title: 'Beta' },
      ]),
    });

    await fetchSessions(client, 'sb-1');

    const sessions = getSessions('sb-1');
    expect(sessions).toHaveLength(2);
    expect(sessions[0].activeRunId).toBe('run-a');
    expect(getActiveSessionId('sb-1')).toBe('a1');
    expect(getLoading('sb-1')).toBe(false);
    expect(getError('sb-1')).toBeNull();
  });

  it('sets error state on API failure', async () => {
    const client = mockClient({
      listChatSessions: vi.fn().mockRejectedValue(new Error('Network error')),
    });

    await fetchSessions(client, 'sb-1');

    expect(getError('sb-1')).toBe('Network error');
    expect(getLoading('sb-1')).toBe(false);
  });
});

describe('createSessionApi', () => {
  it('creates a session via the API and prepends it', async () => {
    seedSession('sb-1', 'existing');
    const client = mockClient({
      createChatSession: vi.fn().mockResolvedValue({ session_id: 'new-s', title: 'New Chat' }),
    });

    const entry = await createSessionApi(client, 'sb-1');

    expect(entry?.id).toBe('new-s');
    expect(getSessions('sb-1')[0].id).toBe('new-s');
    expect(getActiveSessionId('sb-1')).toBe('new-s');
  });
});

describe('deleteSessionApi', () => {
  it('removes the session optimistically and restores on failure', async () => {
    seedSession('sb-1', 's1', 'First');
    seedSession('sb-1', 's2', 'Second');
    setActiveSession('sb-1', 's2');

    const client = mockClient({
      deleteChatSession: vi.fn().mockRejectedValue(new Error('Delete failed')),
    });

    await deleteSessionApi(client, 'sb-1', 's2');

    expect(getSessions('sb-1')).toHaveLength(2);
    expect(getError('sb-1')).toBe('Delete failed');
    expect(getActiveSessionId('sb-1')).toBe('s2');
  });
});

describe('loadSessionDetail', () => {
  it('loads messages and runs into the session entry', async () => {
    seedSession('sb-1', 's1', 'Initial', false);
    const client = mockClient({
      getChatSession: vi.fn().mockResolvedValue({
        session_id: 's1',
        title: 'Loaded',
        sidecar_session_id: 'sidecar-1',
        active_run_id: 'run-1',
        messages: [
          {
            id: 'm1',
            role: 'user',
            content: 'hello',
            created_at: 111,
            parts: [{ id: 'text-1', type: 'text', text: 'hello' }],
          },
          {
            id: 'm2',
            role: 'assistant',
            content: 'hi',
            created_at: 222,
            parts: [
              { id: 'reason-1', type: 'reasoning', text: 'thinking', time: { start: 1, end: 2 } },
              { id: 'text-2', type: 'text', text: 'hi' },
            ],
          },
        ],
        runs: [
          {
            id: 'run-1',
            session_id: 's1',
            kind: 'prompt',
            status: 'running',
            request_text: 'hello',
            created_at: 111,
          },
        ],
        run_progress: [
          {
            seq: 1,
            run_id: 'run-1',
            status: 'running',
            phase: 'running',
            message: 'Operator started the agent run.',
            timestamp_ms: 333,
          },
        ],
      }),
    });

    await loadSessionDetail(client, 'sb-1', 's1');

    const session = getSessions('sb-1')[0];
    expect(session.title).toBe('Loaded');
    expect(session.sidecarSessionId).toBe('sidecar-1');
    expect(session.activeRunId).toBe('run-1');
    expect(session.detailLoaded).toBe(true);
    expect(session.messages).toHaveLength(2);
    expect(session.runs[0].id).toBe('run-1');
    expect(session.runProgress).toHaveLength(1);
    expect(session.runProgress[0].message).toContain('started');
    expect(session.partMap.m1?.[0]).toMatchObject({ type: 'text', id: 'text-1', text: 'hello' });
    expect(session.partMap.m2?.[0]).toMatchObject({ type: 'reasoning', id: 'reason-1', text: 'thinking' });
    expect(session.partMap.m2?.[1]).toMatchObject({ type: 'text', id: 'text-2', text: 'hi' });
  });
});

describe('applyChatStreamEvent', () => {
  it('applies live run updates and progress entries', () => {
    seedSession('sb-1', 's1', 'Live', true);

    applyChatStreamEvent('sb-1', 's1', {
      type: 'run_started',
      data: {
        id: 'run-1',
        session_id: 's1',
        kind: 'prompt',
        status: 'running',
        request_text: 'hello',
        created_at: 111,
        started_at: 222,
      },
    });
    applyChatStreamEvent('sb-1', 's1', {
      type: 'run_progress',
      data: {
        run_id: 'run-1',
        status: 'running',
        phase: 'running',
        message: 'Operator started the agent run.',
        timestamp_ms: 333,
      },
    });

    const session = getSessions('sb-1')[0];
    expect(session.activeRunId).toBe('run-1');
    expect(session.runs[0].status).toBe('running');
    expect(session.runProgress).toHaveLength(1);
    expect(session.runProgress[0].message).toContain('started');
  });

  it('applies live assistant messages', () => {
    seedSession('sb-1', 's1', 'Live', true);

    applyChatStreamEvent('sb-1', 's1', {
      type: 'assistant_message',
      data: {
        id: 'm1',
        role: 'assistant',
        content: 'hello from stream',
        created_at: 10,
      },
    });

    const session = getSessions('sb-1')[0];
    expect(session.messages).toHaveLength(1);
    expect(session.partMap.m1?.[0]).toMatchObject({ type: 'text', text: 'hello from stream' });
  });

  it('replaces reasoning deltas with the same part id', () => {
    seedSession('sb-1', 's1', 'Live', true);

    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.updated',
      data: {
        info: {
          id: 'm1',
          role: 'assistant',
          time: { created: 10 },
        },
      },
    });

    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.part.updated',
      data: {
        part: {
          id: 'reason-1',
          messageID: 'm1',
          type: 'reasoning',
          text: 'The user',
          time: { start: 10 },
        },
      },
    });
    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.part.updated',
      data: {
        part: {
          id: 'reason-1',
          messageID: 'm1',
          type: 'reasoning',
          text: 'The user wants',
          time: { start: 10 },
        },
      },
    });

    const session = getSessions('sb-1')[0];
    expect(session.partMap.m1).toHaveLength(1);
    expect(session.partMap.m1?.[0]).toMatchObject({
      type: 'reasoning',
      id: 'reason-1',
      text: 'The user wants',
    });
  });

  it('replaces text deltas with the same part id', () => {
    seedSession('sb-1', 's1', 'Live', true);

    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.updated',
      data: {
        info: {
          id: 'm1',
          role: 'assistant',
          time: { created: 10 },
        },
      },
    });

    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.part.updated',
      data: {
        part: {
          id: 'text-1',
          messageID: 'm1',
          type: 'text',
          text: 'hello',
        },
      },
    });
    applyChatStreamEvent('sb-1', 's1', {
      type: 'message.part.updated',
      data: {
        part: {
          id: 'text-1',
          messageID: 'm1',
          type: 'text',
          text: 'hello world',
        },
      },
    });

    const session = getSessions('sb-1')[0];
    expect(session.partMap.m1).toHaveLength(1);
    expect(session.partMap.m1?.[0]).toMatchObject({
      type: 'text',
      id: 'text-1',
      text: 'hello world',
    });
  });
});
