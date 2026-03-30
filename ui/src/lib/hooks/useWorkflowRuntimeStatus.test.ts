import { createElement, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { authStateRef, getTokenMock } = vi.hoisted(() => ({
  authStateRef: {
    current: {
      authCacheKey: '0xwalleta::http://operator:9090',
      cachedToken: 'token-a' as string | null,
      isAuthenticated: true,
      isAuthenticating: false,
      error: null as string | null,
    },
  },
  getTokenMock: vi.fn<() => Promise<string | null>>(async () => 'token-a'),
}));

vi.mock('~/lib/hooks/useOperatorAuth', () => ({
  useOperatorAuth: () => ({
    authCacheKey: authStateRef.current.authCacheKey,
    getCachedToken: () => authStateRef.current.cachedToken,
    getToken: getTokenMock,
    isAuthenticated: authStateRef.current.isAuthenticated,
    isAuthenticating: authStateRef.current.isAuthenticating,
    error: authStateRef.current.error,
  }),
}));

import {
  useWorkflowDetail,
  useWorkflowRuntimeStatus,
  useWorkflowSummaries,
} from './useWorkflowRuntimeStatus';
import { getWorkflowOperatorUrl } from '../workflows';

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 0,
      },
    },
  });

  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(QueryClientProvider, { client: queryClient }, children);
  };
}

describe('useWorkflowRuntimeStatus owner-scoped cache keys', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  const sandboxOperatorUrl = getWorkflowOperatorUrl('sandbox');

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    getTokenMock.mockReset();
    getTokenMock.mockImplementation(async () => authStateRef.current.cachedToken);
    authStateRef.current = {
      authCacheKey: '0xwalleta::http://operator:9090',
      cachedToken: 'token-a',
      isAuthenticated: true,
      isAuthenticating: false,
      error: null,
    };
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('refetches workflow summaries when wallet identity changes even if both sessions are valid', async () => {
    fetchMock.mockResolvedValue(jsonResponse({
      workflows: [{ workflowId: 1, scope: 'sandbox', name: 'Wallet A' }],
    }));

    const { rerender } = renderHook(
      () => useWorkflowSummaries('http://operator:9090'),
      { wrapper: createWrapper() },
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    authStateRef.current = {
      ...authStateRef.current,
      authCacheKey: '0xwalletb::http://operator:9090',
      cachedToken: 'token-b',
    };
    fetchMock.mockResolvedValue(jsonResponse({
      workflows: [{ workflowId: 2, scope: 'sandbox', name: 'Wallet B' }],
    }));

    rerender();

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[0]?.[0]).toBe('http://operator:9090/api/workflows');
    expect(fetchMock.mock.calls[1]?.[0]).toBe('http://operator:9090/api/workflows');
  });

  it('refetches workflow detail when wallet identity changes', async () => {
    fetchMock.mockResolvedValue(jsonResponse({
      workflowId: 7,
      scope: 'sandbox',
      name: 'Wallet A Detail',
      triggerType: 'cron',
      triggerConfig: '* * * * *',
      targetKind: 0,
      targetSandboxId: 'sb-1',
      targetServiceId: 5,
      active: true,
      running: false,
      lastRunAt: null,
      nextRunAt: null,
      latestExecution: null,
      workflowJson: '{}',
      sandboxConfigJson: '{}',
    }));

    const { rerender } = renderHook(
      () => useWorkflowDetail('sandbox', '7'),
      { wrapper: createWrapper() },
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    authStateRef.current = {
      ...authStateRef.current,
      authCacheKey: '0xwalletb::http://operator:9090',
      cachedToken: 'token-b',
    };
    fetchMock.mockResolvedValue(jsonResponse({
      workflowId: 7,
      scope: 'sandbox',
      name: 'Wallet B Detail',
      triggerType: 'cron',
      triggerConfig: '* * * * *',
      targetKind: 0,
      targetSandboxId: 'sb-1',
      targetServiceId: 5,
      active: true,
      running: false,
      lastRunAt: null,
      nextRunAt: null,
      latestExecution: null,
      workflowJson: '{}',
      sandboxConfigJson: '{}',
    }));

    rerender();

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[0]?.[0]).toBe(`${sandboxOperatorUrl}/api/workflows/7/detail`);
    expect(fetchMock.mock.calls[1]?.[0]).toBe(`${sandboxOperatorUrl}/api/workflows/7/detail`);
  });

  it('refetches runtime status when wallet identity changes', async () => {
    fetchMock.mockResolvedValue(jsonResponse({
      workflowId: 9,
      running: false,
      lastRunAt: null,
      nextRunAt: null,
      latestExecution: null,
    }));

    const { rerender } = renderHook(
      () => useWorkflowRuntimeStatus('sandbox', '9'),
      { wrapper: createWrapper() },
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    authStateRef.current = {
      ...authStateRef.current,
      authCacheKey: '0xwalletb::http://operator:9090',
      cachedToken: 'token-b',
    };
    fetchMock.mockResolvedValue(jsonResponse({
      workflowId: 9,
      running: true,
      lastRunAt: 1700000000,
      nextRunAt: 1700000300,
      latestExecution: null,
    }));

    rerender();

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[0]?.[0]).toBe(`${sandboxOperatorUrl}/api/workflows/9`);
    expect(fetchMock.mock.calls[1]?.[0]).toBe(`${sandboxOperatorUrl}/api/workflows/9`);
  });

  it('does not fetch when no cached token exists even if auth identity is known', async () => {
    authStateRef.current = {
      ...authStateRef.current,
      authCacheKey: '0xwalleta::http://operator:9090',
      cachedToken: null,
      isAuthenticated: false,
    };

    renderHook(
      () => useWorkflowSummaries('http://operator:9090'),
      { wrapper: createWrapper() },
    );

    await new Promise((resolve) => setTimeout(resolve, 25));
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
