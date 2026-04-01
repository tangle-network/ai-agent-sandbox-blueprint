import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { SessionMessage, SessionPart } from '@tangle-network/sandbox-ui/types';
import type { SandboxClient } from '~/lib/api/sandboxClient';
import {
  applyChatStreamEvent,
  hasActiveRun,
  loadSessionDetail,
  markRunAccepted,
  renameSession,
  type ChatRunEntry,
  type ChatRunProgressEntry,
  type ChatSessionEntry,
} from '~/lib/stores/chatSessions';

const RECONNECT_DELAYS_MS = [1_000, 2_000, 5_000, 10_000, 15_000];

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
  isReconnecting: boolean;
  isCancelling: boolean;
  error: string | null;
  activeRun: ChatRunEntry | null;
  progress: ChatRunProgressEntry[];
  elapsedMs: number;
  send: (text: string) => Promise<void>;
  sendTask: (text: string) => Promise<void>;
  cancelActiveRun: () => Promise<void>;
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
  const [isReconnecting, setIsReconnecting] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const mountedRef = useRef(true);
  const streamAbortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout>>();
  const reconnectAttemptRef = useRef(0);
  const sessionRef = useRef<ChatSessionEntry | null>(session);
  sessionRef.current = session;

  const cleanupStream = useCallback(() => {
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = undefined;
    }
    if (streamAbortRef.current) {
      streamAbortRef.current.abort();
      streamAbortRef.current = null;
    }
  }, []);

  const connectStream = useCallback(async (attempt: number = 0) => {
    if (!client || !sessionRef.current?.id) {
      return;
    }

    cleanupStream();
    const targetSessionId = sessionRef.current.id;
    const controller = new AbortController();
    streamAbortRef.current = controller;

    try {
      await client.streamChatSession(targetSessionId, {
        signal: controller.signal,
        onOpen: () => {
          if (!mountedRef.current) return;
          setIsReconnecting(false);
          setError(null);
          reconnectAttemptRef.current = 0;
        },
        onEvent: (event) => {
          applyChatStreamEvent(sandboxId, targetSessionId, event);
          if (event.type === 'session.idle') {
            void loadSessionDetail(client, sandboxId, targetSessionId);
          }
        },
      });

      if (!mountedRef.current || controller.signal.aborted) {
        return;
      }

      setIsReconnecting(true);
      reconnectAttemptRef.current = attempt + 1;
      const delay = RECONNECT_DELAYS_MS[Math.min(attempt, RECONNECT_DELAYS_MS.length - 1)];
      reconnectTimerRef.current = setTimeout(() => {
        void connectStream(attempt + 1);
      }, delay);
    } catch (err) {
      if (!mountedRef.current || controller.signal.aborted) {
        return;
      }

      setIsReconnecting(true);
      setError((prev) => prev ?? (err instanceof Error ? err.message : 'Chat stream disconnected'));
      reconnectAttemptRef.current = attempt + 1;
      const delay = RECONNECT_DELAYS_MS[Math.min(attempt, RECONNECT_DELAYS_MS.length - 1)];
      reconnectTimerRef.current = setTimeout(async () => {
        if (!client || !sessionRef.current?.id) return;
        await loadSessionDetail(client, sandboxId, sessionRef.current.id);
        void connectStream(attempt + 1);
      }, delay);
    }
  }, [cleanupStream, client, sandboxId]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      cleanupStream();
    };
  }, [cleanupStream]);

  useEffect(() => {
    setError(null);
    setIsReconnecting(false);
    reconnectAttemptRef.current = 0;

    if (!client || !session?.id) {
      cleanupStream();
      return;
    }

    void connectStream(0);
    return () => {
      cleanupStream();
    };
  }, [cleanupStream, client, connectStream, session?.id]);

  useEffect(() => {
    if (!hasActiveRun(session)) {
      return;
    }
    const interval = setInterval(() => setNowMs(Date.now()), 1_000);
    return () => clearInterval(interval);
  }, [session?.activeRunId]);

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

  const activeRun = useMemo(() => {
    if (!session?.activeRunId) {
      return null;
    }
    return session.runs.find((run) => run.id === session.activeRunId) ?? null;
  }, [session]);

  const progress = useMemo(() => {
    if (!activeRun) {
      return [];
    }
    return session?.runProgress.filter((entry) => entry.runId === activeRun.id) ?? [];
  }, [activeRun, session?.runProgress]);

  const cancelActiveRun = useCallback(async () => {
    if (!client || !session?.id || !activeRun || isCancelling) {
      return;
    }

    setError(null);
    setIsCancelling(true);

    try {
      await client.cancelChatRun(session.id, activeRun.id);
      await loadSessionDetail(client, sandboxId, session.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to cancel the active run');
    } finally {
      setIsCancelling(false);
    }
  }, [activeRun, client, isCancelling, sandboxId, session?.id]);

  const clear = useCallback(() => {
    setError(null);
  }, []);

  const elapsedMs = useMemo(() => {
    if (!activeRun) {
      return 0;
    }
    const end = activeRun.completedAt ?? nowMs;
    return Math.max(0, end - activeRun.createdAt);
  }, [activeRun, nowMs]);

  const isStreaming = useMemo(
    () => submitting || hasActiveRun(session),
    [session, submitting],
  );

  return {
    messages: session?.messages ?? [],
    partMap: session?.partMap ?? {},
    isStreaming,
    isReconnecting,
    isCancelling,
    error,
    activeRun,
    progress,
    elapsedMs,
    send,
    sendTask,
    cancelActiveRun,
    clear,
  };
}
