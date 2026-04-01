import { describe, expect, it } from 'vitest';
import type { SessionMessage, SessionPart } from '@tangle-network/sandbox-ui/types';
import { collectVisibleSessionTimelineParts } from './sessionChatTimeline';

function makeMessage(id: string): SessionMessage {
  return {
    id,
    role: 'assistant',
    time: { created: 1 },
  };
}

describe('sessionChatTimeline', () => {
  it('preserves strict chronological order when expanded', () => {
    const messages = [makeMessage('m1')];
    const partMap: Record<string, SessionPart[]> = {
      m1: [
        { type: 'text', text: 'first text' },
        { type: 'tool', id: 'tool-1', tool: 'write', state: { status: 'completed' } },
        { type: 'reasoning', text: 'thinking', time: { start: 1, end: 2 } },
        { type: 'text', text: 'final text' },
      ],
    };

    const parts = collectVisibleSessionTimelineParts(messages, partMap, false);

    expect(parts.map(({ part }) => part.type === 'tool' ? part.id : part.text))
      .toEqual(['first text', 'tool-1', 'thinking', 'final text']);
  });

  it('keeps only text parts visible when collapsed', () => {
    const messages = [makeMessage('m1')];
    const partMap: Record<string, SessionPart[]> = {
      m1: [
        { type: 'text', text: 'first text' },
        { type: 'tool', id: 'tool-1', tool: 'write', state: { status: 'completed' } },
        { type: 'text', text: 'final text' },
      ],
    };

    const parts = collectVisibleSessionTimelineParts(messages, partMap, true);

    expect(parts.map(({ part }) => part.type === 'text' ? part.text : part.type)).toEqual([
      'first text',
      'final text',
    ]);
  });
});
