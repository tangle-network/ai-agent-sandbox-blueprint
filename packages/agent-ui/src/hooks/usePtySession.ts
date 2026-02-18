import { useState, useEffect, useRef, useCallback } from 'react';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface UsePtySessionOptions {
  /** Base URL of the sidecar (e.g. "http://localhost:9100"). */
  apiUrl: string;
  /** Bearer token for authentication. */
  token: string;
  /** Called with raw PTY output (may contain ANSI escape codes). */
  onData: (data: string) => void;
}

export interface UsePtySessionReturn {
  /** Whether the SSE stream is connected and receiving data. */
  isConnected: boolean;
  /** Connection or API error, if any. */
  error: string | null;
  /** Send a command to the PTY session. */
  sendCommand: (command: string) => Promise<void>;
  /** Tear down and reconnect. */
  reconnect: () => void;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Manages a PTY session against the sidecar terminal API.
 *
 * Protocol:
 * - POST /terminals              → create session → { data: { sessionId } }
 * - GET  /terminals/{id}/stream  → SSE output (raw PTY with ANSI codes)
 * - POST /terminals/{id}/execute → send command   { command: "..." }
 * - DELETE /terminals/{id}       → close session
 */
export function usePtySession({ apiUrl, token, onData }: UsePtySessionOptions): UsePtySessionReturn {
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sessionIdRef = useRef<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout>>();
  const mountedRef = useRef(true);
  const onDataRef = useRef(onData);
  onDataRef.current = onData;

  const cleanup = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = undefined;
    }
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
    // Best-effort delete the PTY session
    if (sessionIdRef.current) {
      const sid = sessionIdRef.current;
      sessionIdRef.current = null;
      fetch(`${apiUrl}/terminals/${sid}`, {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    setIsConnected(false);
  }, [apiUrl, token]);

  const connect = useCallback(async () => {
    cleanup();
    setError(null);

    try {
      // Create PTY session
      const res = await fetch(`${apiUrl}/terminals`, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      });

      if (!res.ok) {
        throw new Error(`Failed to create terminal: ${res.status}`);
      }

      const body = await res.json();
      const sessionId: string = body.data?.sessionId ?? body.sessionId;
      if (!sessionId) throw new Error('No sessionId in response');

      if (!mountedRef.current) return;
      sessionIdRef.current = sessionId;

      // Open SSE stream — EventSource doesn't support custom headers,
      // so pass token as query param
      const streamUrl = `${apiUrl}/terminals/${sessionId}/stream?token=${encodeURIComponent(token)}`;
      const es = new EventSource(streamUrl);
      eventSourceRef.current = es;

      es.onopen = () => {
        if (mountedRef.current) {
          setIsConnected(true);
          setError(null);
        }
      };

      es.onmessage = (event) => {
        if (mountedRef.current && event.data) {
          onDataRef.current(event.data);
        }
      };

      es.onerror = () => {
        if (!mountedRef.current) return;
        es.close();
        eventSourceRef.current = null;
        setIsConnected(false);
        // Retry after 3s
        retryTimerRef.current = setTimeout(() => {
          if (mountedRef.current) connect();
        }, 3000);
      };
    } catch (err) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : 'Terminal connection failed');
        setIsConnected(false);
      }
    }
  }, [apiUrl, token, cleanup]);

  const sendCommand = useCallback(async (command: string) => {
    const sid = sessionIdRef.current;
    if (!sid) return;

    const res = await fetch(`${apiUrl}/terminals/${sid}/execute`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ command }),
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || `Execute failed: ${res.status}`);
    }
  }, [apiUrl, token]);

  // Connect on mount
  useEffect(() => {
    mountedRef.current = true;
    connect();
    return () => {
      mountedRef.current = false;
      cleanup();
    };
  }, [connect, cleanup]);

  return { isConnected, error, sendCommand, reconnect: connect };
}
