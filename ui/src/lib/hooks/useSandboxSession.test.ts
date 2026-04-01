import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { chatSessionsStore } from '~/lib/stores/chatSessions';
import { useSandboxSession } from './useSandboxSession';
import type { SandboxClient, ChatStreamEvent } from '~/lib/api/sandboxClient';

function resetStore() {
  chatSessionsStore.set({ sessions: {}, active: {}, loading: {}, error: {} });
}

function seedSession() {
  chatSessionsStore.set({
    sessions: {
      'sb-1': [{
        id: 'session-1',
        title: 'Test',
        sandboxId: 'sb-1',
        createdAt: Date.now(),
        activeRunId: 'run-1',
        sidecarSessionId: undefined,
        runs: [{
          id: 'run-1',
          kind: 'task',
          status: 'running',
          requestText: 'build app',
          createdAt: Date.now(),
        }],
        runProgress: [],
        messages: [],
        partMap: {},
        detailLoaded: true,
      }],
    },
    active: { 'sb-1': 'session-1' },
    loading: {},
    error: {},
  });
}

describe('useSandboxSession', () => {
  beforeEach(() => {
    resetStore();
    seedSession();
  });

  it('reloads canonical session detail when the session becomes idle', async () => {
    const getChatSession = vi.fn().mockResolvedValue({
      session_id: 'session-1',
      title: 'Test',
      active_run_id: null,
      messages: [{
        id: 'assistant-1',
        role: 'assistant',
        content: 'done',
        created_at: 1,
        completed_at: 10,
        parts: [{
          id: 'reason-1',
          type: 'reasoning',
          text: 'done thinking',
          time: { start: 1, end: 10 },
        }],
      }],
      runs: [{
        id: 'run-1',
        session_id: 'session-1',
        kind: 'task',
        status: 'completed',
        request_text: 'build app',
        created_at: 1,
        completed_at: 10,
      }],
    });

    const client = {
      streamChatSession: vi.fn().mockImplementation(async (_sessionId: string, options: {
        signal?: AbortSignal;
        onOpen?: () => void;
        onEvent: (event: ChatStreamEvent) => void;
      }) => {
        options.onOpen?.();
        options.onEvent({ type: 'session.idle', data: { sessionID: 'session-1' } });
      }),
      getChatSession,
    } as unknown as SandboxClient;

    const session = chatSessionsStore.get().sessions['sb-1'][0];
    renderHook(() => useSandboxSession({ client, session, sandboxId: 'sb-1' }));

    await waitFor(() => {
      expect(getChatSession).toHaveBeenCalledWith('session-1');
    });

    await waitFor(() => {
      const updated = chatSessionsStore.get().sessions['sb-1'][0];
      expect(updated.activeRunId).toBeUndefined();
      expect(updated.partMap['assistant-1']?.[0]).toMatchObject({
        type: 'reasoning',
        id: 'reason-1',
        time: { start: 1, end: 10 },
      });
    });
  });
});
