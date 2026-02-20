import { useCallback, useRef, useState } from 'react';
import type { SessionMessage, SessionPart, TextPart, ToolPart } from '@tangle/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import {
  appendMessage,
  updateParts,
  updateSessionSidecarId,
  renameSession,
  type ChatSessionEntry,
} from '~/lib/stores/chatSessions';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface UseSandboxSessionOptions {
  client: SandboxClient | null;
  session: ChatSessionEntry | null;
  sandboxId: string;
  systemPrompt?: string;
}

export interface UseSandboxSessionResult {
  messages: SessionMessage[];
  partMap: Record<string, SessionPart[]>;
  isStreaming: boolean;
  error: string | null;
  send: (text: string) => Promise<void>;
  sendTask: (text: string) => Promise<void>;
  clear: () => void;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useSandboxSession({
  client,
  session,
  sandboxId,
  systemPrompt,
}: UseSandboxSessionOptions): UseSandboxSessionResult {
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nextId = useRef(0);

  const genId = useCallback(() => {
    nextId.current += 1;
    return `msg-${Date.now()}-${nextId.current}`;
  }, []);

  const addMsg = useCallback(
    (role: 'user' | 'assistant', parts: SessionPart[]): string => {
      if (!session) return '';
      const id = genId();
      const msg: SessionMessage = { id, role, time: { created: Date.now() } };
      appendMessage(sandboxId, session.id, msg, parts);
      return id;
    },
    [session, sandboxId, genId],
  );

  /** Send a prompt-style message (single response). */
  const send = useCallback(
    async (text: string) => {
      if (!client || !session || !text.trim()) return;
      setError(null);

      // Add user message
      const userPart: TextPart = { type: 'text', text };
      addMsg('user', [userPart]);

      // Auto-title: use first message text (truncated)
      if (session.messages.length === 0 && session.title === 'New Chat') {
        const autoTitle = text.length > 40 ? `${text.slice(0, 40)}...` : text;
        renameSession(sandboxId, session.id, autoTitle);
      }

      setIsStreaming(true);
      try {
        const result = await client.prompt(text, systemPrompt, session.sidecarSessionId);

        // Store sidecar session ID for continuity
        if (result.sessionId && result.sessionId !== session.sidecarSessionId) {
          updateSessionSidecarId(sandboxId, session.id, result.sessionId);
        }

        const responsePart: TextPart = { type: 'text', text: result.response };
        addMsg('assistant', [responsePart]);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Request failed';
        setError(msg);
        const errorPart: TextPart = { type: 'text', text: `Error: ${msg}` };
        addMsg('assistant', [errorPart]);
      } finally {
        setIsStreaming(false);
      }
    },
    [client, session, sandboxId, systemPrompt, addMsg],
  );

  /** Send a task-style message (autonomous, multi-turn). */
  const sendTask = useCallback(
    async (text: string) => {
      if (!client || !session || !text.trim()) return;
      setError(null);

      const userPart: TextPart = { type: 'text', text };
      addMsg('user', [userPart]);

      if (session.messages.length === 0 && session.title === 'New Chat') {
        const autoTitle = text.length > 40 ? `${text.slice(0, 40)}...` : text;
        renameSession(sandboxId, session.id, autoTitle);
      }

      setIsStreaming(true);
      try {
        const result = await client.task(text, systemPrompt, session.sidecarSessionId);

        if (result.sessionId && result.sessionId !== session.sidecarSessionId) {
          updateSessionSidecarId(sandboxId, session.id, result.sessionId);
        }

        const responsePart: TextPart = { type: 'text', text: result.response };
        addMsg('assistant', [responsePart]);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Task failed';
        setError(msg);
        const errorPart: TextPart = { type: 'text', text: `Error: ${msg}` };
        addMsg('assistant', [errorPart]);
      } finally {
        setIsStreaming(false);
      }
    },
    [client, session, sandboxId, systemPrompt, addMsg],
  );

  const clear = useCallback(() => {
    setError(null);
    nextId.current = 0;
  }, []);

  return {
    messages: session?.messages ?? [],
    partMap: session?.partMap ?? {},
    isStreaming,
    error,
    send,
    sendTask,
    clear,
  };
}
