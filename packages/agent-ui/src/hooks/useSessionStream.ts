import { useCallback, useEffect, useRef, useState } from 'react';
import type { SessionMessage } from '~/types/message';
import type { SessionPart, TextPart, ToolPart, ReasoningPart } from '~/types/parts';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Minimal session info returned by the listing API. */
export interface SessionInfo {
  id: string;
  title: string;
  parentID?: string;
}

export interface UseSessionStreamOptions {
  /** Base URL of the session proxy (e.g. "http://localhost:9100"). */
  apiUrl: string;
  /** Bearer token for authentication. */
  token: string | null;
  /** Session ID to stream events for. */
  sessionId: string;
  /** Only connect when true. Defaults to true. */
  enabled?: boolean;
}

export interface UseSessionStreamResult {
  /** All messages in the session (fetched + streaming). */
  messages: SessionMessage[];
  /** Part map: messageId → SessionPart[]. */
  partMap: Record<string, SessionPart[]>;
  /** Whether the agent is currently streaming a response. */
  isStreaming: boolean;
  /** Send a text message to the agent. */
  send: (text: string) => Promise<void>;
  /** Abort the current agent execution. */
  abort: () => Promise<void>;
  /** Refetch full message history from the API. */
  refetch: () => Promise<void>;
  /** Connection or stream error, if any. */
  error: string | null;
  /** Whether the SSE connection is active. */
  connected: boolean;
}

// ---------------------------------------------------------------------------
// API message shape (what the sidecar returns)
// ---------------------------------------------------------------------------

interface ApiMessagePart {
  type: string;
  text?: string;
  tool?: string;
  id?: string;
  state?: {
    status?: string;
    input?: unknown;
    output?: unknown;
    error?: string;
    metadata?: Record<string, unknown>;
  };
  time?: { start?: number; end?: number };
}

interface ApiMessage {
  info: { id: string; role: string; timestamp?: string };
  parts: ApiMessagePart[];
  source?: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

let _insertionCounter = 0;

function mapApiMessage(msg: ApiMessage): { message: SessionMessage; parts: SessionPart[] } {
  const created = msg.info.timestamp ? new Date(msg.info.timestamp).getTime() : Date.now();
  const message: SessionMessage = {
    id: msg.info.id,
    role: msg.info.role as 'user' | 'assistant' | 'system',
    time: { created },
    _insertionIndex: _insertionCounter++,
  };

  const parts: SessionPart[] = (msg.parts ?? []).map((p, i) => {
    if (p.type === 'tool' && p.tool) {
      return {
        type: 'tool',
        id: p.id ?? `${msg.info.id}-tool-${i}`,
        tool: p.tool,
        state: {
          status: (p.state?.status as 'pending' | 'running' | 'completed' | 'error') ?? 'completed',
          input: p.state?.input,
          output: p.state?.output,
          error: p.state?.error,
          metadata: p.state?.metadata,
          time: p.time,
        },
      } satisfies ToolPart;
    }
    if (p.type === 'reasoning') {
      return {
        type: 'reasoning',
        text: p.text ?? '',
        time: p.time,
      } satisfies ReasoningPart;
    }
    return { type: 'text', text: p.text ?? '' } satisfies TextPart;
  });

  return { message, parts };
}

async function fetchJson<T>(url: string, token: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
  if (init?.body) headers['Content-Type'] = 'application/json';
  const res = await fetch(url, { ...init, headers: { ...headers, ...init?.headers } });
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${res.statusText}`);
  return res.json();
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Streams chat messages from a sidecar session gateway via SSE.
 *
 * Fetches existing message history on mount, then subscribes to SSE for
 * real-time updates. Maps the sidecar's message format to agent-ui's
 * SessionMessage + partMap format.
 */
export function useSessionStream({
  apiUrl,
  token,
  sessionId,
  enabled = true,
}: UseSessionStreamOptions): UseSessionStreamResult {
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [partMap, setPartMap] = useState<Record<string, SessionPart[]>>({});
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);

  const abortRef = useRef<AbortController | null>(null);
  const streamingMsgIdRef = useRef<string | null>(null);

  // ── Fetch full message history ──────────────────────────────────────

  const refetch = useCallback(async () => {
    if (!token || !sessionId || !apiUrl) return;
    try {
      const url = `${apiUrl}/session/sessions/${encodeURIComponent(sessionId)}/messages?limit=200`;
      const data = await fetchJson<ApiMessage[] | { messages: ApiMessage[] }>(url, token);
      const apiMessages = Array.isArray(data) ? data : (data.messages ?? []);

      const newMessages: SessionMessage[] = [];
      const newPartMap: Record<string, SessionPart[]> = {};

      for (const apiMsg of apiMessages) {
        const { message, parts } = mapApiMessage(apiMsg);
        newMessages.push(message);
        newPartMap[message.id] = parts;
      }

      setMessages(newMessages);
      setPartMap(newPartMap);
      streamingMsgIdRef.current = null;
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to fetch messages';
      setError(msg);
    }
  }, [apiUrl, token, sessionId]);

  // ── SSE connection ──────────────────────────────────────────────────

  const connectSSE = useCallback(async () => {
    if (!token || !sessionId || !apiUrl || !enabled) return;

    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    try {
      const url = `${apiUrl}/session/events?sessionId=${encodeURIComponent(sessionId)}`;
      const res = await fetch(url, {
        headers: { Authorization: `Bearer ${token}` },
        signal: controller.signal,
      });

      if (!res.ok) throw new Error(`SSE connection failed: ${res.status}`);
      setConnected(true);
      setError(null);

      const reader = res.body?.getReader();
      if (!reader) throw new Error('No response body');

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

          let eventType = 'message';
          const dataLines: string[] = [];

          for (const line of frame.split('\n')) {
            if (line.startsWith('event:')) {
              eventType = line.slice(6).trim();
            } else if (line.startsWith('data:')) {
              dataLines.push(line.slice(5).trim());
            }
          }

          if (dataLines.length === 0) continue;

          let parsed: Record<string, unknown>;
          try {
            parsed = JSON.parse(dataLines.join('\n'));
          } catch {
            continue;
          }

          handleSSEEvent(eventType, parsed);
        }
      }
    } catch (err) {
      if ((err as Error).name === 'AbortError') return;
      const msg = err instanceof Error ? err.message : 'SSE connection error';
      setError(msg);
      setConnected(false);

      // Auto-reconnect
      if (!controller.signal.aborted) {
        setTimeout(() => connectSSE(), 3000);
      }
    }
  }, [apiUrl, token, sessionId, enabled]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── SSE event handler ──────────────────────────────────────────────

  const handleSSEEvent = useCallback((type: string, props: Record<string, unknown>) => {
    if (type === 'message.updated') {
      // A new or updated message — create/update the message entry
      const id = (props.id as string) ?? (props.messageId as string) ?? '';
      const role = (props.role as string) ?? 'assistant';
      if (!id) return;

      setMessages((prev) => {
        const exists = prev.some((m) => m.id === id);
        if (exists) return prev;
        return [
          ...prev,
          {
            id,
            role: role as 'user' | 'assistant' | 'system',
            time: { created: Date.now() },
            _insertionIndex: _insertionCounter++,
          },
        ];
      });

      if (role === 'assistant') {
        streamingMsgIdRef.current = id;
        setIsStreaming(true);
      }
    } else if (type === 'message.part.updated') {
      // A part within the current streaming message was updated
      const msgId = streamingMsgIdRef.current;
      if (!msgId) return;

      const partType = (props.type as string) ?? 'text';
      setIsStreaming(true);

      setPartMap((prev) => {
        const existing = prev[msgId] ?? [];
        const updated = [...existing];

        if (partType === 'text') {
          const text = (props.text as string) ?? (props.content as string) ?? '';
          const idx = updated.findIndex((p) => p.type === 'text');
          const textPart: TextPart = { type: 'text', text };
          if (idx >= 0) {
            updated[idx] = textPart;
          } else {
            updated.push(textPart);
          }
        } else if (partType === 'tool') {
          const toolId = (props.id as string) ?? (props.toolId as string) ?? `tool-${Date.now()}`;
          const toolName = (props.tool as string) ?? (props.name as string) ?? 'unknown';
          const state = (props.state as ToolPart['state']) ?? { status: 'running' as const };
          const toolPart: ToolPart = {
            type: 'tool',
            id: toolId,
            tool: toolName,
            state: {
              status: (state.status as ToolPart['state']['status']) ?? 'running',
              input: state.input,
              output: state.output,
              error: state.error,
              metadata: state.metadata,
              time: state.time,
            },
          };
          const idx = updated.findIndex((p) => p.type === 'tool' && (p as ToolPart).id === toolId);
          if (idx >= 0) {
            updated[idx] = toolPart;
          } else {
            updated.push(toolPart);
          }
        } else if (partType === 'reasoning') {
          const text = (props.text as string) ?? '';
          const idx = updated.findIndex((p) => p.type === 'reasoning');
          const reasoningPart: ReasoningPart = { type: 'reasoning', text };
          if (idx >= 0) {
            updated[idx] = reasoningPart;
          } else {
            updated.push(reasoningPart);
          }
        }

        return { ...prev, [msgId]: updated };
      });
    } else if (type === 'session.idle') {
      setIsStreaming(false);
      streamingMsgIdRef.current = null;
      // Refetch to get canonical message state
      refetch();
    } else if (type === 'session.error') {
      setIsStreaming(false);
      streamingMsgIdRef.current = null;
      const errorMsg = (props.error as string) ?? (props.message as string) ?? 'Agent error';
      setError(errorMsg);
      refetch();
    }
  }, [refetch]);

  // ── Send message ───────────────────────────────────────────────────

  const send = useCallback(async (text: string) => {
    if (!token || !sessionId || !apiUrl) return;
    try {
      const url = `${apiUrl}/session/sessions/${encodeURIComponent(sessionId)}/messages`;
      await fetchJson<unknown>(url, token, {
        method: 'POST',
        body: JSON.stringify({ parts: [{ type: 'text', text }] }),
      });
      setIsStreaming(true);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to send message';
      setError(msg);
    }
  }, [apiUrl, token, sessionId]);

  // ── Abort ──────────────────────────────────────────────────────────

  const abort = useCallback(async () => {
    if (!token || !sessionId || !apiUrl) return;
    try {
      const url = `${apiUrl}/session/sessions/${encodeURIComponent(sessionId)}/abort`;
      await fetchJson<unknown>(url, token, { method: 'POST' });
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to abort';
      setError(msg);
    }
  }, [apiUrl, token, sessionId]);

  // ── Lifecycle ──────────────────────────────────────────────────────

  useEffect(() => {
    if (!enabled || !token || !sessionId) return;
    refetch();
    connectSSE();
    return () => {
      abortRef.current?.abort();
      setConnected(false);
    };
  }, [enabled, token, sessionId, refetch, connectSSE]);

  return { messages, partMap, isStreaming, send, abort, refetch, error, connected };
}
