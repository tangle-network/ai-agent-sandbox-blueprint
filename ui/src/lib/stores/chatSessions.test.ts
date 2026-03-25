import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { SessionMessage, SessionPart } from '@tangle-network/agent-ui';
import {
  chatSessionsStore,
  renameSession,
  updateSessionSidecarId,
  appendMessage,
  updateParts,
  setActiveSession,
  getActiveSession,
  getActiveSessionId,
  getSessions,
  getLoading,
  getError,
  fetchSessions,
  createSessionApi,
  deleteSessionApi,
  loadSessionMessages,
} from './chatSessions';
import type { SandboxClient } from '~/lib/api/sandboxClient';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function resetStore() {
  chatSessionsStore.set({ sessions: {}, active: {}, loading: {}, error: {} });
}

/** Seed a session directly into the store (simulates API-fetched state). */
function seedSession(sandboxId: string, id: string, title = 'Test', messagesLoaded = true) {
  const state = chatSessionsStore.get();
  const entry = {
    id,
    title,
    sandboxId,
    createdAt: Date.now(),
    messages: [] as SessionMessage[],
    partMap: {} as Record<string, SessionPart[]>,
    messagesLoaded,
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
    getChatSession: vi.fn().mockResolvedValue({ session_id: 's1', title: 'T', messages: [] }),
    deleteChatSession: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as SandboxClient;
}

// ---------------------------------------------------------------------------

beforeEach(() => {
  resetStore();
});

// ── Synchronous store operations ──

describe('renameSession', () => {
  it('updates the title', () => {
    seedSession('sb-1', 's1', 'Original');
    renameSession('sb-1', 's1', 'Renamed');
    expect(getSessions('sb-1')[0].title).toBe('Renamed');
  });

  it('no-op for unknown session', () => {
    seedSession('sb-1', 's1', 'Original');
    renameSession('sb-1', 'unknown-id', 'Renamed');
    expect(getSessions('sb-1')[0].title).toBe('Original');
  });
});

describe('updateSessionSidecarId', () => {
  it('sets sidecarSessionId on the session', () => {
    seedSession('sb-1', 's1');
    updateSessionSidecarId('sb-1', 's1', 'sidecar-session-abc');
    expect(getSessions('sb-1')[0].sidecarSessionId).toBe('sidecar-session-abc');
  });
});

describe('appendMessage', () => {
  it('appends a message and stores parts', () => {
    seedSession('sb-1', 's1');
    const msg: SessionMessage = { id: 'msg-1', role: 'user', time: { created: Date.now() } };
    const parts: SessionPart[] = [{ type: 'text', text: 'hello' }];
    appendMessage('sb-1', 's1', msg, parts);
    const session = getSessions('sb-1')[0];
    expect(session.messages).toHaveLength(1);
    expect(session.messages[0].id).toBe('msg-1');
    expect(session.partMap['msg-1']).toEqual(parts);
  });

  it('appends multiple messages in order', () => {
    seedSession('sb-1', 's1');
    const msg1: SessionMessage = { id: 'msg-1', role: 'user' };
    const msg2: SessionMessage = { id: 'msg-2', role: 'assistant' };
    appendMessage('sb-1', 's1', msg1, []);
    appendMessage('sb-1', 's1', msg2, []);
    const session = getSessions('sb-1')[0];
    expect(session.messages).toHaveLength(2);
    expect(session.messages[0].id).toBe('msg-1');
    expect(session.messages[1].id).toBe('msg-2');
  });
});

describe('updateParts', () => {
  it('updates parts for an existing message', () => {
    seedSession('sb-1', 's1');
    const msg: SessionMessage = { id: 'msg-1', role: 'assistant' };
    appendMessage('sb-1', 's1', msg, [{ type: 'text', text: 'initial' }]);
    updateParts('sb-1', 's1', 'msg-1', [{ type: 'text', text: 'updated' }]);
    expect((getSessions('sb-1')[0].partMap['msg-1'][0] as { text: string }).text).toBe('updated');
  });
});

describe('setActiveSession', () => {
  it('switches the active session', () => {
    seedSession('sb-1', 's1', 'First');
    seedSession('sb-1', 's2', 'Second');
    setActiveSession('sb-1', 's1');
    expect(getActiveSessionId('sb-1')).toBe('s1');
    expect(getActiveSession('sb-1')?.title).toBe('First');
  });
});

describe('getSessions', () => {
  it('returns empty array for unknown sandboxId', () => {
    expect(getSessions('sb-unknown')).toEqual([]);
  });
});

describe('getActiveSession', () => {
  it('returns undefined when no sessions exist', () => {
    expect(getActiveSession('sb-unknown')).toBeUndefined();
  });

  it('returns undefined when active id does not match any session', () => {
    seedSession('sb-1', 's1');
    chatSessionsStore.set({
      ...chatSessionsStore.get(),
      active: { 'sb-1': 'nonexistent' },
    });
    expect(getActiveSession('sb-1')).toBeUndefined();
  });
});

// ── Async API-backed functions ──

describe('fetchSessions', () => {
  it('populates store from API response', async () => {
    const client = mockClient({
      listChatSessions: vi.fn().mockResolvedValue([
        { session_id: 'a1', title: 'Alpha' },
        { session_id: 'b2', title: 'Beta' },
      ]),
    });

    await fetchSessions(client, 'sb-1');

    const sessions = getSessions('sb-1');
    expect(sessions).toHaveLength(2);
    expect(sessions[0].id).toBe('a1');
    expect(sessions[0].title).toBe('Alpha');
    expect(sessions[1].id).toBe('b2');
    expect(getActiveSessionId('sb-1')).toBe('a1');
    expect(getLoading('sb-1')).toBe(false);
    expect(getError('sb-1')).toBeNull();
  });

  it('sets error state on API failure', async () => {
    const client = mockClient({
      listChatSessions: vi.fn().mockRejectedValue(new Error('Network error')),
    });

    await fetchSessions(client, 'sb-1');

    expect(getSessions('sb-1')).toHaveLength(0);
    expect(getError('sb-1')).toBe('Network error');
    expect(getLoading('sb-1')).toBe(false);
  });

  it('skips if already loading', async () => {
    chatSessionsStore.set({
      ...chatSessionsStore.get(),
      loading: { 'sb-1': true },
    });
    const client = mockClient();
    await fetchSessions(client, 'sb-1');
    expect(client.listChatSessions).not.toHaveBeenCalled();
  });

  it('preserves active session if it still exists in new list', async () => {
    seedSession('sb-1', 'keep-me');
    setActiveSession('sb-1', 'keep-me');

    const client = mockClient({
      listChatSessions: vi.fn().mockResolvedValue([
        { session_id: 'new-one', title: 'New' },
        { session_id: 'keep-me', title: 'Kept' },
      ]),
    });

    await fetchSessions(client, 'sb-1');
    expect(getActiveSessionId('sb-1')).toBe('keep-me');
  });
});

describe('createSessionApi', () => {
  it('creates session via API and prepends to store', async () => {
    seedSession('sb-1', 'existing');
    const client = mockClient({
      createChatSession: vi.fn().mockResolvedValue({ session_id: 'new-s', title: 'New Chat' }),
    });

    const entry = await createSessionApi(client, 'sb-1');

    expect(entry).not.toBeNull();
    expect(entry!.id).toBe('new-s');
    const sessions = getSessions('sb-1');
    expect(sessions[0].id).toBe('new-s');
    expect(sessions[1].id).toBe('existing');
    expect(getActiveSessionId('sb-1')).toBe('new-s');
  });

  it('sets error on API failure', async () => {
    const client = mockClient({
      createChatSession: vi.fn().mockRejectedValue(new Error('Create failed')),
    });

    const entry = await createSessionApi(client, 'sb-1');

    expect(entry).toBeNull();
    expect(getError('sb-1')).toBe('Create failed');
  });
});

describe('deleteSessionApi', () => {
  it('removes session optimistically and calls API', async () => {
    seedSession('sb-1', 's1', 'First');
    seedSession('sb-1', 's2', 'Second');
    setActiveSession('sb-1', 's2');

    const client = mockClient({ deleteChatSession: vi.fn().mockResolvedValue(undefined) });

    await deleteSessionApi(client, 'sb-1', 's2');

    expect(getSessions('sb-1')).toHaveLength(1);
    expect(getSessions('sb-1')[0].id).toBe('s1');
    expect(getActiveSessionId('sb-1')).toBe('s1');
    expect(client.deleteChatSession).toHaveBeenCalledWith('s2');
  });

  it('restores session on API failure', async () => {
    seedSession('sb-1', 's1', 'Only');

    const client = mockClient({
      deleteChatSession: vi.fn().mockRejectedValue(new Error('Delete failed')),
    });

    await deleteSessionApi(client, 'sb-1', 's1');

    // Session should be restored
    expect(getSessions('sb-1')).toHaveLength(1);
    expect(getSessions('sb-1')[0].id).toBe('s1');
    expect(getError('sb-1')).toBe('Delete failed');
  });
});

describe('loadSessionMessages', () => {
  it('fetches messages from API and populates session', async () => {
    seedSession('sb-1', 's1', 'Test', false);

    const client = mockClient({
      getChatSession: vi.fn().mockResolvedValue({
        session_id: 's1',
        title: 'Test',
        messages: [
          { role: 'user', content: 'hello' },
          { role: 'assistant', content: 'hi there' },
        ],
      }),
    });

    await loadSessionMessages(client, 'sb-1', 's1');

    const session = getSessions('sb-1')[0];
    expect(session.messagesLoaded).toBe(true);
    expect(session.messages).toHaveLength(2);
    expect(session.messages[0].role).toBe('user');
    expect(session.messages[1].role).toBe('assistant');
    expect((session.partMap['server-0'][0] as { text: string }).text).toBe('hello');
    expect((session.partMap['server-1'][0] as { text: string }).text).toBe('hi there');
  });

  it('sets error on API failure', async () => {
    seedSession('sb-1', 's1', 'Test', false);

    const client = mockClient({
      getChatSession: vi.fn().mockRejectedValue(new Error('Load failed')),
    });

    await loadSessionMessages(client, 'sb-1', 's1');

    expect(getError('sb-1')).toBe('Load failed');
  });
});
