import { describe, it, expect, beforeEach } from 'vitest';
import type { SessionMessage, SessionPart } from '@tangle/agent-ui';
import {
  chatSessionsStore,
  createSession,
  deleteSession,
  renameSession,
  updateSessionSidecarId,
  appendMessage,
  updateParts,
  setActiveSession,
  getActiveSession,
  getActiveSessionId,
  getSessions,
} from './chatSessions';

beforeEach(() => {
  chatSessionsStore.set({ sessions: {}, active: {} });
  localStorage.removeItem('chat_sessions');
});

// ── createSession ──

describe('createSession', () => {
  it('creates a session with default title', () => {
    const entry = createSession('sb-1');
    expect(entry.title).toBe('New Chat');
    expect(entry.sandboxId).toBe('sb-1');
    expect(entry.messages).toEqual([]);
    expect(entry.partMap).toEqual({});
  });

  it('creates a session with custom title', () => {
    const entry = createSession('sb-1', 'My Chat');
    expect(entry.title).toBe('My Chat');
  });

  it('sets the created session as active', () => {
    const entry = createSession('sb-1');
    expect(getActiveSessionId('sb-1')).toBe(entry.id);
  });

  it('prepends new sessions (most recent first)', () => {
    const first = createSession('sb-1', 'First');
    const second = createSession('sb-1', 'Second');
    const sessions = getSessions('sb-1');
    expect(sessions[0].id).toBe(second.id);
    expect(sessions[1].id).toBe(first.id);
  });

  it('isolates sessions by sandboxId', () => {
    createSession('sb-1');
    createSession('sb-2');
    expect(getSessions('sb-1')).toHaveLength(1);
    expect(getSessions('sb-2')).toHaveLength(1);
  });
});

// ── deleteSession ──

describe('deleteSession', () => {
  it('removes the session', () => {
    const entry = createSession('sb-1');
    deleteSession('sb-1', entry.id);
    expect(getSessions('sb-1')).toHaveLength(0);
  });

  it('activates next session when active is deleted', () => {
    const first = createSession('sb-1', 'First');
    createSession('sb-1', 'Second');
    // Second is active and first in list
    const secondId = getActiveSessionId('sb-1')!;
    deleteSession('sb-1', secondId);
    // Should fall back to the remaining session
    expect(getActiveSessionId('sb-1')).toBe(first.id);
  });

  it('clears active when last session is deleted', () => {
    const entry = createSession('sb-1');
    deleteSession('sb-1', entry.id);
    expect(getActiveSessionId('sb-1')).toBe('');
  });

  it('preserves active when non-active session is deleted', () => {
    const first = createSession('sb-1', 'First');
    const second = createSession('sb-1', 'Second');
    // second is active
    deleteSession('sb-1', first.id);
    expect(getActiveSessionId('sb-1')).toBe(second.id);
  });
});

// ── renameSession ──

describe('renameSession', () => {
  it('updates the title', () => {
    const entry = createSession('sb-1');
    renameSession('sb-1', entry.id, 'Renamed');
    expect(getSessions('sb-1')[0].title).toBe('Renamed');
  });

  it('no-op for unknown session', () => {
    createSession('sb-1', 'Original');
    renameSession('sb-1', 'unknown-id', 'Renamed');
    expect(getSessions('sb-1')[0].title).toBe('Original');
  });
});

// ── updateSessionSidecarId ──

describe('updateSessionSidecarId', () => {
  it('sets sidecarSessionId on the session', () => {
    const entry = createSession('sb-1');
    updateSessionSidecarId('sb-1', entry.id, 'sidecar-session-abc');
    expect(getSessions('sb-1')[0].sidecarSessionId).toBe('sidecar-session-abc');
  });
});

// ── appendMessage ──

describe('appendMessage', () => {
  it('appends a message and stores parts', () => {
    const entry = createSession('sb-1');
    const msg: SessionMessage = { id: 'msg-1', role: 'user', time: { created: Date.now() } };
    const parts: SessionPart[] = [{ type: 'text', text: 'hello' }];
    appendMessage('sb-1', entry.id, msg, parts);
    const session = getSessions('sb-1')[0];
    expect(session.messages).toHaveLength(1);
    expect(session.messages[0].id).toBe('msg-1');
    expect(session.partMap['msg-1']).toEqual(parts);
  });

  it('appends multiple messages in order', () => {
    const entry = createSession('sb-1');
    const msg1: SessionMessage = { id: 'msg-1', role: 'user' };
    const msg2: SessionMessage = { id: 'msg-2', role: 'assistant' };
    appendMessage('sb-1', entry.id, msg1, []);
    appendMessage('sb-1', entry.id, msg2, []);
    const session = getSessions('sb-1')[0];
    expect(session.messages).toHaveLength(2);
    expect(session.messages[0].id).toBe('msg-1');
    expect(session.messages[1].id).toBe('msg-2');
  });
});

// ── updateParts ──

describe('updateParts', () => {
  it('updates parts for an existing message', () => {
    const entry = createSession('sb-1');
    const msg: SessionMessage = { id: 'msg-1', role: 'assistant' };
    appendMessage('sb-1', entry.id, msg, [{ type: 'text', text: 'initial' }]);
    updateParts('sb-1', entry.id, 'msg-1', [{ type: 'text', text: 'updated' }]);
    expect((getSessions('sb-1')[0].partMap['msg-1'][0] as { text: string }).text).toBe('updated');
  });
});

// ── setActiveSession / getActiveSession ──

describe('setActiveSession', () => {
  it('switches the active session', () => {
    const first = createSession('sb-1', 'First');
    const second = createSession('sb-1', 'Second');
    // second is active now
    setActiveSession('sb-1', first.id);
    expect(getActiveSessionId('sb-1')).toBe(first.id);
    expect(getActiveSession('sb-1')?.title).toBe('First');
  });
});

// ── getSessions ──

describe('getSessions', () => {
  it('returns empty array for unknown sandboxId', () => {
    expect(getSessions('sb-unknown')).toEqual([]);
  });
});

// ── getActiveSession ──

describe('getActiveSession', () => {
  it('returns undefined when no sessions exist', () => {
    expect(getActiveSession('sb-unknown')).toBeUndefined();
  });

  it('returns undefined when active id does not match any session', () => {
    createSession('sb-1');
    // Manually set a bogus active
    chatSessionsStore.set({
      ...chatSessionsStore.get(),
      active: { 'sb-1': 'nonexistent' },
    });
    expect(getActiveSession('sb-1')).toBeUndefined();
  });
});

// ── localStorage persistence ──

describe('persistence', () => {
  it('persists to localStorage on change', () => {
    createSession('sb-1', 'Persisted');
    const raw = localStorage.getItem('chat_sessions');
    expect(raw).toBeTruthy();
    const parsed = JSON.parse(raw!);
    expect(parsed.sessions['sb-1']).toHaveLength(1);
    expect(parsed.sessions['sb-1'][0].title).toBe('Persisted');
  });
});
