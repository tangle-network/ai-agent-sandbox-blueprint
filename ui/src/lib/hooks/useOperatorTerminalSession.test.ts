import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOperatorTerminalSession } from './useOperatorTerminalSession';

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function errorResponse(status: number, text = '') {
  return new Response(text, { status });
}

function jsonErrorResponse(status: number, error: string, code?: string) {
  return new Response(JSON.stringify({
    error,
    ...(code ? { code } : {}),
  }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function createSseStream() {
  let controller: ReadableStreamDefaultController<Uint8Array> | null = null;
  const stream = new ReadableStream<Uint8Array>({
    start(ctrl) {
      controller = ctrl;
    },
  });

  return {
    response: new Response(stream, { status: 200 }),
    push(message: string, event = 'message') {
      controller?.enqueue(new TextEncoder().encode(`event: ${event}\ndata: ${message}\n\n`));
    },
    close() {
      controller?.close();
    },
  };
}

const BASE_URL = 'http://operator:9090';
const RESOURCE_PATH = '/api/sandboxes/sb-1';
const SESSIONS_URL = `${BASE_URL}${RESOURCE_PATH}/live/terminal/sessions`;
const TERMINAL_SIZE = { cols: 80, rows: 24 };

describe('useOperatorTerminalSession', () => {
  const onOutput = vi.fn();
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onOutput.mockReset();
    fetchMock = vi.fn().mockResolvedValue(jsonResponse({}));
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function renderTerminalHook(initialCwd = '', terminalSize = TERMINAL_SIZE) {
    return renderHook(() => useOperatorTerminalSession({
      apiUrl: BASE_URL,
      resourcePath: RESOURCE_PATH,
      token: 'token-1',
      initialCwd,
      terminalSize,
      onOutput,
    }));
  }

  function mockEmptyListThenCreate(sse: ReturnType<typeof createSseStream>) {
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [] }))
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))
      .mockResolvedValueOnce(sse.response);
  }

  function mockExistingSession(sse: ReturnType<typeof createSseStream>, sessionId = 'existing-1') {
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [{ session_id: sessionId }] }))
      .mockResolvedValueOnce(sse.response);
  }

  it('creates a new session with cwd and size when list returns empty', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(fetchMock.mock.calls[0][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[1][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[1][1]?.method).toBe('POST');
    expect(JSON.parse(String(fetchMock.mock.calls[1][1]?.body))).toEqual({
      cwd: '/home/agent',
      cols: 80,
      rows: 24,
    });
    expect(fetchMock.mock.calls[2][0]).toBe(`${SESSIONS_URL}/term-1/stream`);
    expect(result.current.sessionId).toBe('term-1');

    unmount();
  });

  it('reconnects to an existing session and patches its size', async () => {
    const sse = createSseStream();
    mockExistingSession(sse);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));
    await waitFor(() => {
      expect(fetchMock.mock.calls.some(
        (call) => call[0] === `${SESSIONS_URL}/existing-1` && call[1]?.method === 'PATCH',
      )).toBe(true);
    });

    expect(fetchMock.mock.calls[0][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[1][0]).toBe(`${SESSIONS_URL}/existing-1/stream`);
    const resizeCall = fetchMock.mock.calls.find(
      (call) => call[0] === `${SESSIONS_URL}/existing-1` && call[1]?.method === 'PATCH',
    );
    expect(resizeCall?.[1]?.body).toBe(JSON.stringify(TERMINAL_SIZE));
    expect(result.current.sessionId).toBe('existing-1');

    unmount();
  });

  it('falls back to create when reconnecting to a stale session fails', async () => {
    const sse = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [{ session_id: 'stale-1' }] }))
      .mockResolvedValueOnce(errorResponse(404, 'not found'))
      .mockResolvedValueOnce(jsonResponse({ deleted: true }))
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-new' }))
      .mockResolvedValueOnce(sse.response);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    expect(fetchMock.mock.calls[2][0]).toBe(`${SESSIONS_URL}/stale-1`);
    expect(fetchMock.mock.calls[2][1]?.method).toBe('DELETE');
    expect(fetchMock.mock.calls[3][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[3][1]?.method).toBe('POST');
    expect(JSON.parse(String(fetchMock.mock.calls[3][1]?.body))).toEqual({
      cwd: '/home/agent',
      cols: 80,
      rows: 24,
    });
    expect(result.current.sessionId).toBe('term-new');

    unmount();
  });

  it('does not delete the session on unmount', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    const callCountBefore = fetchMock.mock.calls.length;
    unmount();

    const postUnmountCalls = fetchMock.mock.calls.slice(callCountBefore);
    const deleteCalls = postUnmountCalls.filter((call) => call[1]?.method === 'DELETE');
    expect(deleteCalls).toHaveLength(0);
  });

  it('newSession() force-creates a fresh session', async () => {
    const sse1 = createSseStream();
    mockExistingSession(sse1);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));
    expect(result.current.sessionId).toBe('existing-1');

    const sse2 = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'forced-1' }))
      .mockResolvedValueOnce(sse2.response);

    await act(async () => {
      result.current.newSession();
    });

    await waitFor(() => expect(result.current.sessionId).toBe('forced-1'));
    expect(fetchMock.mock.calls.at(-2)?.[0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls.at(-2)?.[1]?.method).toBe('POST');
    expect(JSON.parse(String(fetchMock.mock.calls.at(-2)?.[1]?.body))).toEqual({
      cwd: '/home/agent',
      cols: 80,
      rows: 24,
    });
    expect(fetchMock.mock.calls.at(-1)?.[0]).toBe(`${SESSIONS_URL}/forced-1/stream`);

    unmount();
  });

  it('sends raw terminal input to the session input endpoint', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    fetchMock.mockResolvedValueOnce(jsonResponse({ success: true }));

    await act(async () => {
      await result.current.sendInput('ls -la\n');
    });

    expect(fetchMock.mock.calls.at(-1)?.[0]).toBe(`${SESSIONS_URL}/term-1/input`);
    expect(fetchMock.mock.calls.at(-1)?.[1]?.method).toBe('POST');
    expect(fetchMock.mock.calls.at(-1)?.[1]?.body).toBe(JSON.stringify({ data: 'ls -la\n' }));

    unmount();
  });

  it('parses structured terminal stream events into text output', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    act(() => {
      sse.push(JSON.stringify({
        type: 'data.stdout',
        properties: { text: 'prompt$ ' },
      }), 'data.stdout');
    });

    await waitFor(() => expect(onOutput).toHaveBeenCalledWith('prompt$ '));
    expect(result.current.error).toBeNull();

    unmount();
  });

  it('does not retry when the operator reports terminal PTY support is unavailable', async () => {
    vi.useFakeTimers();
    fetchMock.mockResolvedValueOnce(
      jsonErrorResponse(502, 'Sidecar PTY terminal API is not supported by this sandbox image/runtime.', 'TERMINAL_UNSUPPORTED'),
    );

    const { result, unmount } = renderTerminalHook('/home/agent');
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.error).toBe(
      'Sidecar PTY terminal API is not supported by this sandbox image/runtime.',
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('does not recreate an existing session when the stream failure is terminal unsupported', async () => {
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [{ session_id: 'existing-1' }] }))
      .mockResolvedValueOnce(
        jsonErrorResponse(502, 'Sidecar PTY terminal API is not supported by this sandbox image/runtime.', 'TERMINAL_UNSUPPORTED'),
      );

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => {
      expect(result.current.error).toBe(
        'Sidecar PTY terminal API is not supported by this sandbox image/runtime.',
      );
    });

    expect(fetchMock).toHaveBeenCalledTimes(2);
    const deleteCalls = fetchMock.mock.calls.filter((call) => call[1]?.method === 'DELETE');
    const createCalls = fetchMock.mock.calls.filter((call) => call[1]?.method === 'POST');
    expect(deleteCalls).toHaveLength(0);
    expect(createCalls).toHaveLength(0);

    unmount();
  });
});
