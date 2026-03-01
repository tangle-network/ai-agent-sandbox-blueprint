import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useExposedPorts } from './useExposedPorts';

type OperatorApiCall = (
  action: string,
  body?: Record<string, unknown>,
  opts?: { method?: string },
) => Promise<Response>;

let mockApiCall: ReturnType<typeof vi.fn<OperatorApiCall>>;

const MOCK_PORTS = [
  { container_port: 3000, host_port: 32001, protocol: 'tcp' },
  { container_port: 8080, host_port: 32002, protocol: 'tcp' },
];

beforeEach(() => {
  mockApiCall = vi.fn();
});

function setup(status: string | undefined) {
  return renderHook(
    ({ status: s }) => useExposedPorts(s, mockApiCall),
    { initialProps: { status } },
  );
}

describe('status gating', () => {
  it('does not fetch when status is undefined', () => {
    setup(undefined);
    expect(mockApiCall).not.toHaveBeenCalled();
  });

  it('does not fetch when status is stopped', () => {
    setup('stopped');
    expect(mockApiCall).not.toHaveBeenCalled();
  });

  it('does not fetch when status is gone', () => {
    setup('gone');
    expect(mockApiCall).not.toHaveBeenCalled();
  });

  it('fetches when status is running', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_PORTS)));
    const { result } = setup('running');
    expect(mockApiCall).toHaveBeenCalledWith('ports', undefined, { method: 'GET' });
    await waitFor(() => expect(result.current).not.toBeNull());
  });

  it('fetches when status is creating', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_PORTS)));
    const { result } = setup('creating');
    expect(mockApiCall).toHaveBeenCalledWith('ports', undefined, { method: 'GET' });
    await waitFor(() => expect(result.current).not.toBeNull());
  });
});

describe('initial state', () => {
  it('returns null initially', () => {
    const { result } = setup('stopped');
    expect(result.current).toBeNull();
  });
});

describe('successful fetch', () => {
  it('returns port data after fetch resolves', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify(MOCK_PORTS)));
    const { result } = setup('running');

    await waitFor(() => {
      expect(result.current).toEqual(MOCK_PORTS);
    });
  });
});

describe('error handling', () => {
  it('silently ignores fetch errors (returns null)', async () => {
    mockApiCall.mockRejectedValue(new Error('Not found'));
    const { result } = setup('running');

    // Give the effect time to settle — should not throw
    await new Promise((r) => setTimeout(r, 50));
    expect(result.current).toBeNull();
  });

  it('ignores non-array response data', async () => {
    mockApiCall.mockResolvedValue(new Response(JSON.stringify({ error: 'not found' })));
    const { result } = setup('running');

    await new Promise((r) => setTimeout(r, 50));
    expect(result.current).toBeNull();
  });
});

describe('cleanup', () => {
  it('does not set state after unmount', async () => {
    let resolvePromise: (v: Response) => void;
    mockApiCall.mockReturnValue(new Promise<Response>((r) => { resolvePromise = r; }));

    const { result, unmount } = setup('running');
    unmount();

    // Resolve after unmount — should not throw or update state
    resolvePromise!(new Response(JSON.stringify(MOCK_PORTS)));
    await new Promise((r) => setTimeout(r, 50));

    // result.current is the last value before unmount — still null
    expect(result.current).toBeNull();
  });
});
