import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { sandboxListStore, type LocalSandbox } from '~/lib/stores/sandboxes';

const { sandboxAuth, instanceAuth, toastError } = vi.hoisted(() => ({
  sandboxAuth: {
    getToken: vi.fn(),
    getCachedToken: vi.fn(),
    isAuthenticated: false,
    isAuthenticating: false,
    error: null as string | null,
  },
  instanceAuth: {
    getToken: vi.fn(),
    getCachedToken: vi.fn(),
    isAuthenticated: false,
    isAuthenticating: false,
    error: null as string | null,
  },
  toastError: vi.fn(),
}));

vi.mock('./useOperatorAuth', () => ({
  useOperatorAuth: (apiUrl?: string) => (apiUrl === 'http://instance:9200' ? instanceAuth : sandboxAuth),
}));

vi.mock('~/lib/config', () => ({
  OPERATOR_API_URL: 'http://operator:9100',
  INSTANCE_OPERATOR_API_URL: 'http://instance:9200',
}));

vi.mock('sonner', () => ({
  toast: {
    error: toastError,
  },
}));

import { useSandboxHydration } from './useSandboxHydration';

function makeLocalSandbox(overrides: Partial<LocalSandbox> = {}): LocalSandbox {
  return {
    localId: 'legacy:sandbox-1',
    sandboxId: undefined,
    name: 'sandbox-1',
    image: 'agent-dev:latest',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: '1',
    serviceId: '1',
    sidecarUrl: '',
    status: 'creating',
    ...overrides,
  };
}

function operatorSandbox(id = 'sandbox-live-1') {
  return {
    id,
    name: 'sb1',
    sidecar_url: 'http://127.0.0.1:57041',
    state: 'running',
    image: 'agent-dev:latest',
    cpu_cores: 2,
    memory_mb: 2048,
    disk_gb: 10,
    created_at: 1700000000,
    last_activity_at: 1700000100,
  };
}

describe('useSandboxHydration hook', () => {
  beforeEach(() => {
    sandboxListStore.set([]);
    sandboxAuth.getToken.mockReset();
    sandboxAuth.getCachedToken.mockReset();
    instanceAuth.getToken.mockReset();
    instanceAuth.getCachedToken.mockReset();
    toastError.mockReset();
    vi.stubGlobal('fetch', vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('passively hydrates from the operator using a cached token', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValue('cached-token');
    instanceAuth.getCachedToken.mockReturnValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [operatorSandbox()] }),
    } as Response);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(sandboxListStore.get()).toHaveLength(1);
      expect(sandboxListStore.get()[0].name).toBe('sb1');
    });

    expect(result.current.authRequired).toBe(false);
    expect(sandboxAuth.getToken).not.toHaveBeenCalled();
    expect(fetchMock).toHaveBeenCalledWith('http://operator:9100/api/sandboxes', {
      headers: { Authorization: 'Bearer cached-token' },
      signal: expect.any(AbortSignal),
    });
  });

  it('keeps cached rows and marks authRequired when no cached token exists', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValue(null);
    instanceAuth.getCachedToken.mockReturnValue(null);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    expect(sandboxListStore.get()).toHaveLength(1);
    expect(sandboxListStore.get()[0].name).toBe('sandbox-1');
    expect(sandboxAuth.getToken).not.toHaveBeenCalled();
    expect(vi.mocked(fetch)).not.toHaveBeenCalled();
    expect(toastError).not.toHaveBeenCalled();
  });

  it('interactive refresh authenticates and replaces stale local rows with operator truth', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValue(null);
    sandboxAuth.getToken.mockResolvedValue('interactive-token');
    instanceAuth.getCachedToken.mockReturnValue(null);
    instanceAuth.getToken.mockResolvedValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [operatorSandbox()] }),
    } as Response);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    await act(async () => {
      await result.current.refresh({ interactive: true });
    });

    expect(sandboxAuth.getToken).toHaveBeenCalled();
    expect(result.current.authRequired).toBe(false);
    expect(sandboxListStore.get()).toHaveLength(1);
    expect(sandboxListStore.get()[0].name).toBe('sb1');
    expect(sandboxListStore.get()[0].sandboxId).toBe('sandbox-live-1');
  });

  it('interactive refresh keeps unmatched tx-backed drafts alongside canonical operator sandboxes', async () => {
    sandboxListStore.set([
      makeLocalSandbox({
        localId: 'draft:tx-pending',
        txHash: '0xabc',
        createdAt: Date.now(),
      }),
    ]);
    sandboxAuth.getCachedToken.mockReturnValue(null);
    sandboxAuth.getToken.mockResolvedValue('interactive-token');
    instanceAuth.getCachedToken.mockReturnValue(null);
    instanceAuth.getToken.mockResolvedValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [operatorSandbox('sandbox-live-2')] }),
    } as Response);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    await act(async () => {
      await result.current.refresh({ interactive: true });
    });

    expect(sandboxListStore.get()).toHaveLength(2);
    expect(sandboxListStore.get()[0].localId).toBe('draft:tx-pending');
    expect(sandboxListStore.get()[0].sandboxId).toBeUndefined();
    expect(sandboxListStore.get()[1].localId).toBe('canonical:sandbox-live-2');
    expect(sandboxListStore.get()[1].sandboxId).toBe('sandbox-live-2');
    expect(sandboxListStore.get()[1].name).toBe('sb1');
  });

  it('passive hydration prunes stale canonical sandboxes once operator truth is available', async () => {
    sandboxListStore.set([
      makeLocalSandbox({
        localId: 'canonical:sandbox-stale',
        sandboxId: 'sandbox-stale',
        name: 'stale',
        status: 'stopped',
      }),
      makeLocalSandbox({
        localId: 'canonical:sandbox-live-1',
        sandboxId: 'sandbox-live-1',
        name: 'live',
        status: 'stopped',
      }),
    ]);
    sandboxAuth.getCachedToken.mockReturnValue('cached-token');
    instanceAuth.getCachedToken.mockReturnValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [operatorSandbox('sandbox-live-1')] }),
    } as Response);

    renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(sandboxListStore.get()).toHaveLength(1);
      expect(sandboxListStore.get()[0].sandboxId).toBe('sandbox-live-1');
    });
  });

  it('interactive refresh surfaces auth cancellation instead of silently keeping cached data', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValue(null);
    sandboxAuth.getToken.mockResolvedValue(null);
    instanceAuth.getCachedToken.mockReturnValue(null);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    await act(async () => {
      await result.current.refresh({ interactive: true });
    });

    expect(result.current.lastError).toBe('Operator authentication was cancelled or failed');
    expect(toastError).toHaveBeenCalledWith('Operator authentication was cancelled or failed', {
      description: 'Sign the wallet challenge to refresh sandbox state from the operator.',
      duration: 6000,
    });
    expect(sandboxListStore.get()[0].name).toBe('sandbox-1');
  });

  it('interactive refresh surfaces operator request failures', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValue(null);
    sandboxAuth.getToken.mockResolvedValue('interactive-token');
    instanceAuth.getCachedToken.mockReturnValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: false,
      status: 500,
      text: async () => 'operator exploded',
    } as Response);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    await act(async () => {
      await result.current.refresh({ interactive: true });
    });

    expect(result.current.lastError).toBe('operator exploded');
    expect(toastError).toHaveBeenCalledWith('Unable to refresh sandboxes', {
      description: 'operator exploded',
      duration: 6000,
    });
  });

  it('retries passive hydration when the page becomes visible again', async () => {
    sandboxListStore.set([makeLocalSandbox()]);
    sandboxAuth.getCachedToken.mockReturnValueOnce(null).mockReturnValue('cached-later');
    instanceAuth.getCachedToken.mockReturnValue(null);

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ sandboxes: [operatorSandbox('sandbox-visible-1')] }),
    } as Response);

    const { result } = renderHook(() => useSandboxHydration());

    await waitFor(() => {
      expect(result.current.authRequired).toBe(true);
    });

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });

    await waitFor(() => {
      expect(sandboxListStore.get()[0].sandboxId).toBe('sandbox-visible-1');
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
