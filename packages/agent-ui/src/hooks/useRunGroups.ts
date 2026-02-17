import { useMemo } from 'react';
import type { SessionMessage } from '~/types/message';
import type { SessionPart, ToolPart } from '~/types/parts';
import type { Run, RunStats, GroupedMessage, ToolCategory } from '~/types/run';
import { getToolCategory } from '~/utils/toolDisplay';

// ---------------------------------------------------------------------------
// Run grouping algorithm
//
// Groups consecutive assistant messages into Runs. User messages break the
// sequence and start a new potential run.
// ---------------------------------------------------------------------------

function computeRunStats(
  messages: SessionMessage[],
  partMap: Record<string, SessionPart[]>,
): RunStats {
  const stats: RunStats = {
    toolCount: 0,
    messageCount: messages.length,
    thinkingDurationMs: 0,
    textPartCount: 0,
    toolCategories: new Set(),
  };

  for (const msg of messages) {
    const parts = partMap[msg.id] ?? [];
    for (const part of parts) {
      switch (part.type) {
        case 'tool':
          stats.toolCount++;
          stats.toolCategories.add(getToolCategory(part.tool));
          break;
        case 'text':
          if (!part.synthetic) stats.textPartCount++;
          break;
        case 'reasoning': {
          const start = part.time?.start;
          const end = part.time?.end;
          if (start && end) stats.thinkingDurationMs += end - start;
          break;
        }
      }
    }
  }
  return stats;
}

function getLastTextContent(
  messages: SessionMessage[],
  partMap: Record<string, SessionPart[]>,
): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const parts = partMap[messages[i].id] ?? [];
    for (let j = parts.length - 1; j >= 0; j--) {
      const part = parts[j];
      if (part.type === 'text' && !part.synthetic && part.text.trim()) {
        return part.text.trim();
      }
    }
  }
  return null;
}

export interface UseRunGroupsOptions {
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
  isStreaming: boolean;
}

export function useRunGroups({
  messages,
  partMap,
  isStreaming,
}: UseRunGroupsOptions): GroupedMessage[] {
  return useMemo(() => {
    const groups: GroupedMessage[] = [];
    let currentRunMessages: SessionMessage[] = [];

    function flushRun(streaming: boolean) {
      if (currentRunMessages.length === 0) return;
      const msgs = [...currentRunMessages];
      const stats = computeRunStats(msgs, partMap);
      const summaryText = getLastTextContent(msgs, partMap);
      const isComplete = !streaming;

      groups.push({
        type: 'run',
        run: {
          id: msgs[0].id,
          messages: msgs,
          isComplete,
          isStreaming: streaming,
          stats,
          summaryText,
          finalTextPart: null,
        },
      });
      currentRunMessages = [];
    }

    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      if (msg.role === 'user') {
        flushRun(false);
        groups.push({ type: 'user', message: msg });
      } else {
        currentRunMessages.push(msg);
      }
    }

    // Flush remaining — last run may still be streaming
    if (currentRunMessages.length > 0) {
      flushRun(isStreaming);
    }

    return groups;
  }, [messages, partMap, isStreaming]);
}
