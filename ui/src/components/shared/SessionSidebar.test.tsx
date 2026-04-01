import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { chatSessionsStore } from '~/lib/stores/chatSessions';
import { SessionSidebar } from './SessionSidebar';

function resetStore() {
  chatSessionsStore.set({
    sessions: {},
    active: {},
    loading: {},
    error: {},
  });
}

beforeEach(() => {
  resetStore();
});

describe('SessionSidebar', () => {
  it('preserves partial assistant output and renders an inline failed state', () => {
    chatSessionsStore.set({
      sessions: {
        'sb-1': [
          {
            id: 'session-1',
            title: 'Test Chat',
            sandboxId: 'sb-1',
            createdAt: 1,
            sidecarSessionId: 'sidecar-1',
            activeRunId: undefined,
            runs: [
              {
                id: 'run-1',
                kind: 'prompt',
                status: 'failed',
                requestText: 'hello',
                createdAt: 1,
                completedAt: 2,
                error: 'Agent stream failed',
              },
            ],
            runProgress: [],
            messages: [
              {
                id: 'user-1',
                role: 'user',
                time: { created: 1 },
              },
              {
                id: 'assistant-1',
                role: 'assistant',
                runId: 'run-1',
                success: false,
                error: 'Agent stream failed',
                time: { created: 2, completed: 3 },
              },
            ],
            partMap: {
              'user-1': [{ type: 'text', text: 'hello' }],
              'assistant-1': [{ type: 'text', text: 'Partial answer before failure' }],
            },
            detailLoaded: true,
          },
        ],
      },
      active: { 'sb-1': 'session-1' },
      loading: {},
      error: {},
    });

    render(<SessionSidebar sandboxId="sb-1" client={null} />);

    expect(screen.getByText('Partial answer before failure')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(
      screen.getByText('Generation stopped due to an error. This response may be incomplete.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Agent stream failed')).toBeInTheDocument();
  });
});
