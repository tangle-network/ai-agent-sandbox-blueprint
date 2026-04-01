import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AppMarkdown, ReasoningRow, ToolRow, UserBubble } from './SessionChatParts';

describe('SessionChatParts', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders user messages with local markdown formatting', () => {
    render(
      <UserBubble
        parts={[
          {
            type: 'text',
            text: 'Hello **team** with `code`',
          },
        ]}
      />,
    );

    expect(screen.getByText('You')).toBeInTheDocument();
    expect(screen.getByText(/Hello/i)).toBeInTheDocument();
    expect(screen.getByText('team')).toBeInTheDocument();
    expect(screen.getByText('code')).toBeInTheDocument();
  });

  it('renders basic markdown blocks without sandbox-ui styles', () => {
    render(<AppMarkdown>{'# Heading\n\n- first\n- second'}</AppMarkdown>);

    expect(screen.getByText('Heading')).toBeInTheDocument();
    expect(screen.getByText('first')).toBeInTheDocument();
    expect(screen.getByText('second')).toBeInTheDocument();
  });

  it('shows tool details when a tool row is expanded', () => {
    render(
      <ToolRow
        part={{
          type: 'tool',
          id: 'tool-1',
          tool: 'bash',
          state: {
            status: 'completed',
            input: { command: 'ls -la' },
            output: { stdout: 'file-a\nfile-b' },
            time: { start: 10, end: 3010 },
          },
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: /run command/i }));

    expect(screen.getByText('Input')).toBeInTheDocument();
    expect(screen.getByText('Output')).toBeInTheDocument();
    expect(screen.getAllByText(/ls -la/i)).toHaveLength(2);
  });

  it('auto-collapses completed reasoning after a short delay', () => {
    vi.useFakeTimers();
    const fullReasoning =
      'Thinking through the next step carefully with enough detail to exceed the preview threshold and confirm that collapse behavior still keeps a short summary visible.';

    render(
      <ReasoningRow
        defaultOpen
        part={{
          type: 'reasoning',
          text: fullReasoning,
          time: { start: 100, end: 1200 },
        }}
      />,
    );

    expect(screen.getByText(fullReasoning)).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(950);
    });

    expect(screen.queryByText(fullReasoning)).not.toBeInTheDocument();
    expect(screen.getByText(/Thinking through the next step carefully/i)).toBeInTheDocument();
  });
});
