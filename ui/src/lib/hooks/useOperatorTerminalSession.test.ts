import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOperatorTerminalSession } from './useOperatorTerminalSession';

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
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
    push(message: string) {
      controller?.enqueue(new TextEncoder().encode(`data: ${message}\n\n`));
    },
    close() {
      controller?.close();
    },
  };
}

describe('useOperatorTerminalSession', () => {
  const onOutput = vi.fn();
  const onCommandComplete = vi.fn();
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    onOutput.mockReset();
    onCommandComplete.mockReset();
    fetchMock = vi.fn().mockResolvedValue(jsonResponse({ deleted: true }));
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function renderTerminalHook() {
    return renderHook(() => useOperatorTerminalSession({
      apiUrl: 'http://operator:9090',
      resourcePath: '/api/sandboxes/sb-1',
      token: 'token-1',
      onOutput,
      onCommandComplete,
    }));
  }

  it('falls back to exec response output when the stream stays silent', async () => {
    const sse = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))
      .mockResolvedValueOnce(sse.response)
      .mockResolvedValueOnce(jsonResponse({ stdout: 'file-a\nfile-b\n' }));

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

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

    unmount();
  });

  it('prefers stream output and suppresses the response fallback when stream data arrives in time', async () => {
    const sse = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))
      .mockResolvedValueOnce(sse.response)
      .mockResolvedValueOnce(jsonResponse({ stdout: 'stream-result\n' }));

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

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
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))
      .mockResolvedValueOnce(sse.response)
      .mockResolvedValueOnce(jsonResponse({ stdout: '', stderr: '' }));

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

    await act(async () => {
      await result.current.sendCommand('true');
    });

    expect(onOutput).not.toHaveBeenCalled();
    expect(onCommandComplete).toHaveBeenCalledTimes(1);

    unmount();
  });

  it('suppresses a late duplicate stream chunk after fallback output already printed', async () => {
    const sse = createSseStream();
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ session_id: 'term-1' }))
      .mockResolvedValueOnce(sse.response)
      .mockResolvedValueOnce(jsonResponse({ stdout: 'late-result\n' }));

    const { result, unmount } = renderTerminalHook();
    await waitFor(() => expect(result.current.isConnected).toBe(true));

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
