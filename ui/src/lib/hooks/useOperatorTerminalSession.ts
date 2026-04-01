import { useCallback, useEffect, useRef, useState } from 'react';

export interface UseOperatorTerminalSessionOptions {
  apiUrl: string;
  resourcePath: string;
  token: string;
  initialCwd?: string;
  onOutput: (data: string) => void;
  onCommandComplete: () => void;
}

export interface UseOperatorTerminalSessionReturn {
  isConnected: boolean;
  error: string | null;
  sessionId: string | null;
  sendCommand: (command: string) => Promise<void>;
  reconnect: () => void;
  newSession: () => void;
}

interface TerminalSessionResponse {
  session_id?: string;
  sessionId?: string;
}

interface TerminalSessionListResponse {
  sessions?: Array<{ session_id?: string; sessionId?: string; title?: string }>;
}

interface ExecResponse {
  stdout?: string;
  stderr?: string;
}

interface PendingCommand {
  id: number;
  streamSeen: boolean;
  fallbackTimer?: ReturnType<typeof setTimeout>;
  settleTimer?: ReturnType<typeof setTimeout>;
}

const STREAM_FALLBACK_MS = 150;
const STREAM_SETTLE_MS = 40;
const LATE_STREAM_DEDUPE_MS = 1000;
const KEEP_ALIVE_MESSAGE = 'keep-alive';

function parseSseFrames(chunk: string): string[] {
  const messages: string[] = [];
  let eventData: string[] = [];

  for (const line of chunk.split('\n')) {
    if (!line.trim()) {
      if (eventData.length > 0) {
        messages.push(eventData.join('\n'));
        eventData = [];
      }
      continue;
    }

    if (line.startsWith('data:')) {
      eventData.push(line.slice(5).trimStart());
    }
  }

  return messages;
}

export function useOperatorTerminalSession({
  apiUrl,
  resourcePath,
  token,
  initialCwd = '',
  onOutput,
  onCommandComplete,
}: UseOperatorTerminalSessionOptions): UseOperatorTerminalSessionReturn {
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);

  const sessionIdRef = useRef<string | null>(null);
  const streamAbortRef = useRef<AbortController | null>(null);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout>>();
  const mountedRef = useRef(true);
  const nextCommandIdRef = useRef(1);
  const pendingCommandRef = useRef<PendingCommand | null>(null);
  const recentFallbackChunksRef = useRef<Map<string, number>>(new Map());
  const onOutputRef = useRef(onOutput);
  const onCommandCompleteRef = useRef(onCommandComplete);
  const resolvedInitialCwd = initialCwd.trim();

  onOutputRef.current = onOutput;
  onCommandCompleteRef.current = onCommandComplete;

  const terminalSessionBaseUrl = `${apiUrl}${resourcePath}/live/terminal/sessions`;
  const execUrl = `${apiUrl}${resourcePath}/exec`;

  const clearCommandTimer = useCallback((timer?: ReturnType<typeof setTimeout>) => {
    if (timer) clearTimeout(timer);
  }, []);

  const clearPendingCommand = useCallback((pending: PendingCommand | null) => {
    if (!pending) return;
    clearCommandTimer(pending.fallbackTimer);
    clearCommandTimer(pending.settleTimer);
  }, [clearCommandTimer]);

  const finishPendingCommand = useCallback((commandId: number) => {
    const pending = pendingCommandRef.current;
    if (!pending || pending.id !== commandId) return;

    clearPendingCommand(pending);
    pendingCommandRef.current = null;
    if (mountedRef.current) {
      onCommandCompleteRef.current();
    }
  }, [clearPendingCommand]);

  const rememberFallbackChunk = useCallback((chunk: string) => {
    if (!chunk) return;
    recentFallbackChunksRef.current.set(chunk, Date.now() + LATE_STREAM_DEDUPE_MS);
  }, []);

  const shouldSuppressLateStreamChunk = useCallback((chunk: string) => {
    const now = Date.now();
    for (const [value, expiresAt] of recentFallbackChunksRef.current.entries()) {
      if (expiresAt <= now) {
        recentFallbackChunksRef.current.delete(value);
      }
    }

    const expiresAt = recentFallbackChunksRef.current.get(chunk);
    if (!expiresAt) return false;

    recentFallbackChunksRef.current.delete(chunk);
    return expiresAt > now;
  }, []);

  const emitOutput = useCallback((data: string) => {
    if (!data) return;
    onOutputRef.current(data);
  }, []);

  const handleStreamMessage = useCallback((message: string) => {
    if (!message || message === KEEP_ALIVE_MESSAGE) return;
    if (shouldSuppressLateStreamChunk(message)) return;

    emitOutput(message);

    const pending = pendingCommandRef.current;
    if (!pending) {
      onCommandCompleteRef.current();
      return;
    }

    pending.streamSeen = true;
    clearCommandTimer(pending.fallbackTimer);
    clearCommandTimer(pending.settleTimer);
    pending.settleTimer = setTimeout(() => {
      finishPendingCommand(pending.id);
    }, STREAM_SETTLE_MS);
  }, [clearCommandTimer, emitOutput, finishPendingCommand, shouldSuppressLateStreamChunk]);

  const cleanupStream = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = undefined;
    }
    if (streamAbortRef.current) {
      streamAbortRef.current.abort();
      streamAbortRef.current = null;
    }
    clearPendingCommand(pendingCommandRef.current);
    pendingCommandRef.current = null;
    recentFallbackChunksRef.current.clear();
    setIsConnected(false);
  }, [clearPendingCommand]);

  const connectToStream = useCallback(async (targetSessionId: string) => {
    sessionIdRef.current = targetSessionId;
    if (mountedRef.current) {
      setSessionId(targetSessionId);
    }

    const controller = new AbortController();
    streamAbortRef.current = controller;

    const streamRes = await fetch(
      `${terminalSessionBaseUrl}/${encodeURIComponent(targetSessionId)}/stream`,
      {
        headers: { Authorization: `Bearer ${token}` },
        signal: controller.signal,
      },
    );

    if (!streamRes.ok) {
      const text = await streamRes.text();
      throw new Error(text || `Terminal stream failed: ${streamRes.status}`);
    }

    if (!streamRes.body) {
      throw new Error('Terminal stream is unavailable');
    }

    if (mountedRef.current) {
      setIsConnected(true);
      setError(null);
    }

    const reader = streamRes.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const frames = buffer.split('\n\n');
      buffer = frames.pop() ?? '';

      for (const frame of frames) {
        if (!frame.trim()) continue;
        for (const message of parseSseFrames(frame)) {
          handleStreamMessage(message);
        }
      }
    }
  }, [handleStreamMessage, terminalSessionBaseUrl, token]);

  const createSession = useCallback(async (): Promise<string> => {
    const createRes = await fetch(terminalSessionBaseUrl, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({}),
    });

    if (!createRes.ok) {
      const text = await createRes.text();
      throw new Error(text || `Failed to create terminal session: ${createRes.status}`);
    }

    const body = await createRes.json() as TerminalSessionResponse;
    const id = body.session_id ?? body.sessionId;
    if (!id) throw new Error('Missing terminal session id');
    return id;
  }, [terminalSessionBaseUrl, token]);

  const resolveAndConnect = useCallback(async () => {
    cleanupStream();
    setError(null);

    try {
      try {
        const listRes = await fetch(terminalSessionBaseUrl, {
          headers: { Authorization: `Bearer ${token}` },
        });

        if (listRes.ok) {
          const body = await listRes.json() as TerminalSessionListResponse;
          const sessions = body.sessions ?? [];

          if (sessions.length > 0) {
            const last = sessions[sessions.length - 1];
            const existingId = last.session_id ?? last.sessionId;

            if (existingId) {
              try {
                await connectToStream(existingId);
                return;
              } catch (streamErr) {
                if ((streamErr as Error).name === 'AbortError' || !mountedRef.current) {
                  return;
                }

                fetch(`${terminalSessionBaseUrl}/${encodeURIComponent(existingId)}`, {
                  method: 'DELETE',
                  headers: { Authorization: `Bearer ${token}` },
                }).catch(() => {});
              }
            }
          }
        }
      } catch {
        // Listing failed, so we fall back to creating a fresh session.
      }

      if (!mountedRef.current) return;

      const newId = await createSession();
      if (!mountedRef.current) return;
      await connectToStream(newId);
    } catch (err) {
      if ((err as Error).name === 'AbortError' || !mountedRef.current) return;

      setIsConnected(false);
      setError(err instanceof Error ? err.message : 'Terminal connection failed');
      retryTimerRef.current = setTimeout(() => {
        if (mountedRef.current) {
          void resolveAndConnect();
        }
      }, 3000);
    }
  }, [cleanupStream, connectToStream, createSession, terminalSessionBaseUrl, token]);

  const forceNewSession = useCallback(async () => {
    cleanupStream();
    setError(null);

    try {
      const newId = await createSession();
      if (!mountedRef.current) return;
      await connectToStream(newId);
    } catch (err) {
      if ((err as Error).name === 'AbortError' || !mountedRef.current) return;

      setIsConnected(false);
      setError(err instanceof Error ? err.message : 'Terminal connection failed');
      retryTimerRef.current = setTimeout(() => {
        if (mountedRef.current) {
          void forceNewSession();
        }
      }, 3000);
    }
  }, [cleanupStream, connectToStream, createSession]);

  const sendCommand = useCallback(async (command: string) => {
    const sid = sessionIdRef.current;
    if (!sid) {
      throw new Error('Terminal session is not connected');
    }

    clearPendingCommand(pendingCommandRef.current);
    const commandId = nextCommandIdRef.current++;
    pendingCommandRef.current = {
      id: commandId,
      streamSeen: false,
    };

    try {
      const res = await fetch(execUrl, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          command,
          session_id: sid,
          ...(resolvedInitialCwd ? { cwd: resolvedInitialCwd } : {}),
        }),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `Command failed: ${res.status}`);
      }

      const execBody = await res.json() as ExecResponse;
      const fallbackChunks = [
        execBody.stdout ?? '',
        execBody.stderr ? `[stderr] ${execBody.stderr}` : '',
      ].filter(Boolean);

      const pending = pendingCommandRef.current;
      if (!pending || pending.id !== commandId) return;
      if (pending.streamSeen) return;

      if (fallbackChunks.length === 0) {
        finishPendingCommand(commandId);
        return;
      }

      pending.fallbackTimer = setTimeout(() => {
        const current = pendingCommandRef.current;
        if (!current || current.id !== commandId || current.streamSeen) return;

        for (const chunk of fallbackChunks) {
          rememberFallbackChunk(chunk);
          emitOutput(chunk);
        }

        finishPendingCommand(commandId);
      }, STREAM_FALLBACK_MS);
    } catch (err) {
      clearPendingCommand(pendingCommandRef.current);
      pendingCommandRef.current = null;
      throw err;
    }
  }, [
    clearPendingCommand,
    emitOutput,
    execUrl,
    finishPendingCommand,
    rememberFallbackChunk,
    resolvedInitialCwd,
    token,
  ]);

  useEffect(() => {
    mountedRef.current = true;
    void resolveAndConnect();

    return () => {
      mountedRef.current = false;
      cleanupStream();
    };
  }, [cleanupStream, resolveAndConnect]);

  return {
    isConnected,
    error,
    sessionId,
    sendCommand,
    reconnect: () => {
      void resolveAndConnect();
    },
    newSession: () => {
      void forceNewSession();
    },
  };
}
