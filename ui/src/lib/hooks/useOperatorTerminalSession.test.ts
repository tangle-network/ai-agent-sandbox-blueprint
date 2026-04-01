import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOperatorTerminalSession } from '@tangle-network/sandbox-ui';

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function errorResponse(status: number, text = '') {
  return new Response(text, { status });
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
    push(message: string) {
      controller?.enqueue(new TextEncoder().encode(`data: ${message}\n\n`));
    },
    close() {
      controller?.close();
    },
  };
}

const BASE_URL = 'http://operator:9090';
const RESOURCE_PATH = '/api/sandboxes/sb-1';
const SESSIONS_URL = `${BASE_URL}${RESOURCE_PATH}/live/terminal/sessions`;

describe('useOperatorTerminalSession', () => {
  const onOutput = vi.fn();
  const onCommandComplete = vi.fn();
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    onOutput.mockReset();
    onCommandComplete.mockReset();
    fetchMock = vi.fn().mockResolvedValue(jsonResponse({}));
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function renderTerminalHook(initialCwd = '') {
    return renderHook(() => useOperatorTerminalSession({
      apiUrl: BASE_URL,
      resourcePath: RESOURCE_PATH,
      token: 'token-1',
      initialCwd,
      onOutput,
      onCommandComplete,
    }));
  }

  /** Helper: mock list returning empty, then create + stream */
  function mockEmptyListThenCreate(sse: ReturnType<typeof createSseStream>) {
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [] }))           // GET list
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))   // POST create
      .mockResolvedValueOnce(sse.response);                            // GET stream
  }

  /** Helper: mock list returning an existing session, then stream */
  function mockExistingSession(sse: ReturnType<typeof createSseStream>, sessionId = 'existing-1') {
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [{ session_id: sessionId }] }))  // GET list
      .mockResolvedValueOnce(sse.response);                                             // GET stream (reconnect)
  }

  // ---------------------------------------------------------------------------
  // Session resolution
  // ---------------------------------------------------------------------------

  it('creates a new session when list returns empty', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    // Verify: GET list, POST create, GET stream
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(fetchMock.mock.calls[0][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[0][1]?.method).toBeUndefined(); // GET
    expect(fetchMock.mock.calls[1][0]).toBe(SESSIONS_URL);
    expect(fetchMock.mock.calls[1][1]?.method).toBe('POST');

    expect(result.current.sessionId).toBe('term-1');
    unmount();
  });

  it('reconnects to existing session when list returns sessions', async () => {
    const sse = createSseStream();
    mockExistingSession(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    // Verify: GET list, GET stream (no POST create)
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(result.current.sessionId).toBe('existing-1');
    unmount();
  });

  it('falls back to create when reconnecting to stale session fails', async () => {
    const sse = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ sessions: [{ session_id: 'stale-1' }] }))  // GET list
      .mockResolvedValueOnce(errorResponse(404, 'not found'))                           // GET stream (fails)
      .mockResolvedValueOnce(jsonResponse({ deleted: true }))                           // DELETE stale (fire-and-forget)
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-new' }))                  // POST create
      .mockResolvedValueOnce(sse.response);                                             // GET stream

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    expect(result.current.sessionId).toBe('term-new');
    unmount();
  });

  it('does not DELETE session on unmount', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    const callCountBefore = fetchMock.mock.calls.length;
    unmount();

    // No DELETE call after unmount
    const postUnmountCalls = fetchMock.mock.calls.slice(callCountBefore);
    const deleteCalls = postUnmountCalls.filter(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (c: any) => c[1]?.method === 'DELETE',
    );
    expect(deleteCalls).toHaveLength(0);
  });

  it('newSession() force-creates regardless of existing sessions', async () => {
    const sse1 = createSseStream();
    mockExistingSession(sse1);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));
    expect(result.current.sessionId).toBe('existing-1');

    // Now force new session
    const sse2 = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'forced-1' }))  // POST create
      .mockResolvedValueOnce(sse2.response);                            // GET stream

    await act(async () => {
      result.current.newSession();
    });

    await waitFor(() => expect(result.current.sessionId).toBe('forced-1'));
    unmount();
  });

  // ---------------------------------------------------------------------------
  // Command execution (preserved from original tests)
  // ---------------------------------------------------------------------------

  it('falls back to exec response output when the stream stays silent', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook('/home/agent');
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    fetchMock.mockResolvedValueOnce(jsonResponse({ stdout: 'file-a\nfile-b\n' }));

    let commandPromise!: Promise<void>;
    act(() => {
      commandPromise = result.current.sendCommand('ls');
    });

    await act(async () => {
      await commandPromise;
    });

    await act(async () => {
      vi.advanceTimersByTime(200);
    });

    expect(onOutput).toHaveBeenCalledTimes(1);
    expect(onOutput).toHaveBeenCalledWith('file-a\nfile-b\n');
    expect(onCommandComplete).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls.at(-1)?.[0]).toBe(`${BASE_URL}${RESOURCE_PATH}/exec`);
    expect(fetchMock.mock.calls.at(-1)?.[1]?.body).toBe(
      JSON.stringify({
        command: 'ls',
        session_id: 'term-1',
        cwd: '/home/agent',
      }),
    );

    unmount();
  });

  it('prefers stream output and suppresses the response fallback when stream data arrives in time', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    fetchMock.mockResolvedValueOnce(jsonResponse({ stdout: 'stream-result\n' }));

    let commandPromise!: Promise<void>;
    act(() => {
      commandPromise = result.current.sendCommand('ls');
    });

    await act(async () => {
      await commandPromise;
    });

    await act(async () => {
      sse.push('stream-result\n');
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(onOutput).toHaveBeenCalledTimes(1);
      expect(onOutput).toHaveBeenCalledWith('stream-result\n');
      expect(onCommandComplete).toHaveBeenCalledTimes(1);
    });

    await act(async () => {
      vi.advanceTimersByTime(200);
    });

    expect(onOutput).toHaveBeenCalledTimes(1);
    expect(onCommandComplete).toHaveBeenCalledTimes(1);

    unmount();
  });

  it('restores command completion even when exec returns no output', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    fetchMock.mockResolvedValueOnce(jsonResponse({ stdout: '', stderr: '' }));

    await act(async () => {
      await result.current.sendCommand('true');
    });

    expect(onOutput).not.toHaveBeenCalled();
    expect(onCommandComplete).toHaveBeenCalledTimes(1);

    unmount();
  });

  it('suppresses a late duplicate stream chunk after fallback output already printed', async () => {
    const sse = createSseStream();
    mockEmptyListThenCreate(sse);

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    fetchMock.mockResolvedValueOnce(jsonResponse({ stdout: 'late-result\n' }));

    let commandPromise!: Promise<void>;
    act(() => {
      commandPromise = result.current.sendCommand('ls');
    });

    await act(async () => {
      await commandPromise;
    });

    await act(async () => {
      vi.advanceTimersByTime(200);
    });

    expect(onOutput).toHaveBeenCalledTimes(1);
    expect(onCommandComplete).toHaveBeenCalledTimes(1);

    await act(async () => {
      sse.push('late-result\n');
      await Promise.resolve();
      vi.advanceTimersByTime(100);
    });

    expect(onOutput).toHaveBeenCalledTimes(1);
    expect(onCommandComplete).toHaveBeenCalledTimes(1);

    unmount();
  });
});
