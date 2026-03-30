import { useCallback, useMemo, useState } from 'react';
import type { SessionMessage, SessionPart } from '@tangle-network/agent-ui';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import {
  hasActiveRun,
  loadSessionDetail,
  markRunAccepted,
  renameSession,
  type ChatSessionEntry,
} from '~/lib/stores/chatSessions';

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

export function useSandboxSession({
  client,
  session,
  sandboxId,
  systemPrompt,
}: UseSandboxSessionOptions): UseSandboxSessionResult {
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submitRun = useCallback(
    async (kind: 'prompt' | 'task', text: string) => {
      if (!client || !session || !text.trim()) return;
      setError(null);
      setSubmitting(true);

      if (session.messages.length === 0 && session.title === 'New Chat') {
        const autoTitle = text.length > 40 ? `${text.slice(0, 40)}...` : text;
        renameSession(sandboxId, session.id, autoTitle);
      }

      try {
        const result = kind === 'prompt'
          ? await client.prompt(text, systemPrompt, session.id)
          : await client.task(text, systemPrompt, session.id);

        if (!result.runId || !result.sessionId) {
          throw new Error('Chat submission succeeded but did not return a run identifier');
        }

        markRunAccepted(
          sandboxId,
          result.sessionId,
          result.runId,
          (result.status as ChatSessionEntry['runs'][number]['status']) ?? 'queued',
          result.acceptedAt ?? Date.now(),
          kind,
          text,
        );
        await loadSessionDetail(client, sandboxId, result.sessionId);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Chat submission failed');
      } finally {
        setSubmitting(false);
      }
    },
    [client, sandboxId, session, systemPrompt],
  );

  const send = useCallback(async (text: string) => {
    await submitRun('prompt', text);
  }, [submitRun]);

  const sendTask = useCallback(async (text: string) => {
    await submitRun('task', text);
  }, [submitRun]);

  const clear = useCallback(() => {
    setError(null);
  }, []);

  const isStreaming = useMemo(
    () => submitting || hasActiveRun(session),
    [session, submitting],
  );

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
