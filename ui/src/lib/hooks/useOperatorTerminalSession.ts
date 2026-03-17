import { useCallback, useEffect, useRef, useState } from 'react';

export interface UseOperatorTerminalSessionOptions {
  apiUrl: string;
  resourcePath: string;
  token: string;
  onData: (data: string) => void;
}

export interface UseOperatorTerminalSessionReturn {
  isConnected: boolean;
  error: string | null;
  sendCommand: (command: string) => Promise<void>;
  reconnect: () => void;
}

interface TerminalSessionResponse {
  session_id?: string;
  sessionId?: string;
}

interface ExecResponse {
  stdout?: string;
  stderr?: string;
}

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
  onData,
}: UseOperatorTerminalSessionOptions): UseOperatorTerminalSessionReturn {
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sessionIdRef = useRef<string | null>(null);
  const streamAbortRef = useRef<AbortController | null>(null);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout>>();
  const mountedRef = useRef(true);
  const onDataRef = useRef(onData);
  onDataRef.current = onData;

  const terminalSessionBaseUrl = `${apiUrl}${resourcePath}/live/terminal/sessions`;
  const execUrl = `${apiUrl}${resourcePath}/exec`;

  const cleanup = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = undefined;
    }
    if (streamAbortRef.current) {
      streamAbortRef.current.abort();
      streamAbortRef.current = null;
    }
    if (sessionIdRef.current) {
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      fetch(`${terminalSessionBaseUrl}/${encodeURIComponent(sessionId)}`, {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    setIsConnected(false);
  }, [terminalSessionBaseUrl, token]);

  const connect = useCallback(async () => {
    cleanup();
    setError(null);

    try {
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
      const sessionId = body.session_id ?? body.sessionId;
      if (!sessionId) throw new Error('Missing terminal session id');
      if (!mountedRef.current) return;

      sessionIdRef.current = sessionId;

      const controller = new AbortController();
      streamAbortRef.current = controller;

      const streamRes = await fetch(
        `${terminalSessionBaseUrl}/${encodeURIComponent(sessionId)}/stream`,
        {
          headers: { Authorization: `Bearer ${token}` },
          signal: controller.signal,
        },
      );

      if (!streamRes.ok) {
        const text = await streamRes.text();
        throw new Error(text || `Terminal stream failed: ${streamRes.status}`);
      }

      if (!streamRes.body) throw new Error('Terminal stream is unavailable');

      setIsConnected(true);
      setError(null);

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
            if (message) onDataRef.current(message);
          }
        }
      }
    } catch (err) {
      if ((err as Error).name === 'AbortError' || !mountedRef.current) return;
      setIsConnected(false);
      setError(err instanceof Error ? err.message : 'Terminal connection failed');
      retryTimerRef.current = setTimeout(() => {
        if (mountedRef.current) {
          void connect();
        }
      }, 3000);
    }
  }, [cleanup, terminalSessionBaseUrl, token]);

  const sendCommand = useCallback(async (command: string) => {
    const sessionId = sessionIdRef.current;
    if (!sessionId) throw new Error('Terminal session is not connected');

    const res = await fetch(execUrl, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        command,
        session_id: sessionId,
      }),
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || `Command failed: ${res.status}`);
    }

    const execBody = await res.json() as ExecResponse;
    if (!execBody.stdout && !execBody.stderr) {
      onDataRef.current('');
    }
  }, [execUrl, token]);

  useEffect(() => {
    mountedRef.current = true;
    void connect();
    return () => {
      mountedRef.current = false;
      cleanup();
    };
  }, [cleanup, connect]);

  return {
    isConnected,
    error,
    sendCommand,
    reconnect: () => {
      void connect();
    },
  };
}
