import { useCallback, useRef, useState } from 'react';
import type { SessionMessage } from '@tangle/agent-ui';
import type { SessionPart, TextPart, ToolPart } from '@tangle/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface UseSandboxChatOptions {
  client: SandboxClient | null;
  /** Mode determines which API endpoint to call */
  mode: 'terminal' | 'prompt' | 'task';
  systemPrompt?: string;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Chat-style interaction with a sandbox via the SandboxClient.
 * Manages messages + parts state compatible with @tangle/agent-ui components.
 */
export function useSandboxChat({ client, mode, systemPrompt }: UseSandboxChatOptions) {
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [partMap, setPartMap] = useState<Record<string, SessionPart[]>>({});
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nextId = useRef(0);

  const genId = useCallback(() => {
    nextId.current += 1;
    return `msg-${nextId.current}`;
  }, []);

  const addMessage = useCallback(
    (role: 'user' | 'assistant', parts: SessionPart[]) => {
      const id = genId();
      const msg: SessionMessage = { id, role, time: { created: Date.now() } };
      setMessages((prev) => [...prev, msg]);
      setPartMap((prev) => ({ ...prev, [id]: parts }));
      return id;
    },
    [genId],
  );

  const send = useCallback(
    async (text: string) => {
      if (!client || !text.trim()) return;

      setError(null);

      // Add user message
      const userPart: TextPart = { type: 'text', text };
      addMessage('user', [userPart]);

      setIsStreaming(true);

      try {
        switch (mode) {
          case 'terminal': {
            // Create a tool part for the command
            const toolId = genId();
            const toolPart: ToolPart = {
              type: 'tool',
              id: toolId,
              tool: 'bash',
              state: {
                status: 'running',
                input: { command: text },
                time: { start: Date.now() },
              },
            };
            const assistantId = addMessage('assistant', [toolPart]);

            const result = await client.exec(text);

            // Update with result
            const completedTool: ToolPart = {
              ...toolPart,
              state: {
                ...toolPart.state,
                status: result.exitCode === 0 ? 'completed' : 'error',
                output: result,
                error: result.exitCode !== 0 ? result.stderr || `Exit code: ${result.exitCode}` : undefined,
                time: { start: toolPart.state.time?.start, end: Date.now() },
              },
            };
            setPartMap((prev) => ({ ...prev, [assistantId]: [completedTool] }));
            break;
          }

          case 'prompt': {
            const result = await client.prompt(text, systemPrompt);
            const responsePart: TextPart = { type: 'text', text: result.response };
            addMessage('assistant', [responsePart]);
            break;
          }

          case 'task': {
            const result = await client.task(text, systemPrompt);
            const responsePart: TextPart = { type: 'text', text: result.response };
            addMessage('assistant', [responsePart]);
            break;
          }
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Request failed';
        setError(msg);
        const errorPart: TextPart = { type: 'text', text: `Error: ${msg}` };
        addMessage('assistant', [errorPart]);
      } finally {
        setIsStreaming(false);
      }
    },
    [client, mode, systemPrompt, addMessage, genId],
  );

  const clear = useCallback(() => {
    setMessages([]);
    setPartMap({});
    setError(null);
    nextId.current = 0;
  }, []);

  return {
    messages,
    partMap,
    isStreaming,
    error,
    send,
    clear,
  };
}
