import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// ── Mock wagmi before importing the hook ──

const mockAddress = '0x1234567890abcdef1234567890abcdef12345678';
let currentAddress: string | undefined = mockAddress;
const mockSignMessageAsync = vi.fn();

vi.mock('wagmi', () => ({
  useAccount: () => ({ address: currentAddress }),
  useSignMessage: () => ({ signMessageAsync: mockSignMessageAsync }),
}));

vi.mock('~/lib/config', () => ({
  OPERATOR_API_URL: 'http://test-operator:9090',
}));

import { resetOperatorAuthStoreForTests, useOperatorAuth } from './useOperatorAuth';

// ── Helpers ──

function mockFetchResponses(challenge: object | null, session: object | null) {
  const fetchMock = vi.fn();
  fetchMock.mockImplementation(async (url: string) => {
    if (url.includes('/api/auth/challenge')) {
      if (!challenge) return { ok: false, text: async () => 'Challenge error' };
      return { ok: true, json: async () => challenge };
    }
    if (url.includes('/api/auth/session')) {
      if (!session) return { ok: false, text: async () => 'Session error' };
      return { ok: true, json: async () => session };
    }
    return { ok: false, text: async () => 'Unknown endpoint' };
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

describe('useOperatorAuth', () => {
  beforeEach(() => {
    currentAddress = mockAddress;
    mockSignMessageAsync.mockReset();
    vi.restoreAllMocks();
    resetOperatorAuthStoreForTests();
    window.sessionStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  // ── Initial state ──

  it('starts unauthenticated with no cached session', () => {
    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));
    expect(result.current.isAuthenticated).toBe(false);
    expect(result.current.isAuthenticating).toBe(false);
    expect(result.current.error).toBeNull();
  });

  // ── getToken success flow ──

  it('returns valid PASETO token on successful auth', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc123' },
      { token: 'v4.public.test-token', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBe('v4.public.test-token');
    expect(result.current.error).toBeNull();
  });

  it('reports isAuthenticated=true after getToken when re-rendered', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc123' },
      { token: 'v4.public.test-token', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result, rerender } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    // Force a re-render so isAuthenticated (computed from ref) refreshes.
    // In React 19 act batching, the setIsAuthenticating(true -> false) round-trip
    // may be elided, so the hook may not re-render on its own.
    rerender();

    expect(result.current.isAuthenticated).toBe(true);
  });

  // ── Token expiry validation (60s buffer) ──

  it('considers token invalid when expiry is within 60s buffer', async () => {
    // Token expires in 30 seconds — within the 60s buffer
    const nearExpiry = Math.floor(Date.now() / 1000) + 30;
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.near-expiry', expires_at: nearExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result, rerender } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    rerender();

    // The token was stored but isValid() treats it as expired (within 60s buffer)
    expect(result.current.isAuthenticated).toBe(false);
  });

  it('considers token valid when expiry is well past 60s buffer', async () => {
    const farExpiry = Math.floor(Date.now() / 1000) + 600; // 10 min
    mockFetchResponses(
      { message: 'Sign this', nonce: 'xyz' },
      { token: 'v4.public.valid', expires_at: farExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result, rerender } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    rerender();

    expect(result.current.isAuthenticated).toBe(true);
  });

  it('considers token at exactly 61s from expiry as valid', async () => {
    const borderExpiry = Math.floor(Date.now() / 1000) + 61;
    mockFetchResponses(
      { message: 'Sign this', nonce: 'bdr' },
      { token: 'v4.public.border', expires_at: borderExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result, rerender } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    rerender();

    expect(result.current.isAuthenticated).toBe(true);
  });

  // ── Token caching ──

  it('returns cached token on subsequent calls without re-fetching', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.cached', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    // Reset fetch to track subsequent calls
    fetchMock.mockClear();

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBe('v4.public.cached');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('re-fetches when forceRefresh is true even with valid cached token', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.initial', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    fetchMock.mockClear();
    fetchMock.mockImplementation(async (url: string) => {
      if (url.includes('/api/auth/challenge'))
        return { ok: true, json: async () => ({ message: 'Sign again', nonce: 'def' }) };
      if (url.includes('/api/auth/session'))
        return { ok: true, json: async () => ({ token: 'v4.public.refreshed', expires_at: futureExpiry }) };
      return { ok: false, text: async () => 'Unknown' };
    });

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken(true);
    });

    expect(token).toBe('v4.public.refreshed');
    expect(fetchMock).toHaveBeenCalled();
  });

  // ── Error handling ──

  it('sets error when challenge API fails', async () => {
    mockFetchResponses(null, null);

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBeNull();
    expect(result.current.error).toContain('Challenge failed');
    expect(result.current.isAuthenticated).toBe(false);
  });

  it('sets error when session API fails', async () => {
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      null,
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBeNull();
    expect(result.current.error).toContain('Session creation failed');
  });

  it('sets error when wallet signing fails', async () => {
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'never-reached', expires_at: 0 },
    );
    mockSignMessageAsync.mockRejectedValue(new Error('User rejected'));

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBeNull();
    expect(result.current.error).toBe('User rejected');
  });

  it('surfaces non-Error exceptions as "Auth failed"', async () => {
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'never-reached', expires_at: 0 },
    );
    mockSignMessageAsync.mockRejectedValue('not an Error object');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    expect(result.current.error).toBe('Auth failed');
  });

  it('returns null when no address is connected', async () => {
    currentAddress = undefined;
    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBeNull();
  });

  // ── Session clear on address change ──

  it('clears session when wallet address changes', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.first-addr', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result, rerender } = renderHook(() => useOperatorAuth('http://test:9090'));

    // Authenticate with first address
    await act(async () => {
      await result.current.getToken();
    });

    rerender();
    expect(result.current.isAuthenticated).toBe(true);

    // Simulate address change
    currentAddress = '0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef';
    rerender();

    expect(result.current.isAuthenticated).toBe(false);

    fetchMock.mockClear();
    fetchMock.mockImplementation(async (url: string) => {
      if (url.includes('/api/auth/challenge')) {
        return { ok: true, json: async () => ({ message: 'Sign for new wallet', nonce: 'def' }) };
      }
      if (url.includes('/api/auth/session')) {
        return { ok: true, json: async () => ({ token: 'v4.public.second-addr', expires_at: futureExpiry }) };
      }
      return { ok: false, text: async () => 'Unknown endpoint' };
    });

    let token: string | null = null;
    await act(async () => {
      token = await result.current.getToken();
    });

    expect(token).toBe('v4.public.second-addr');
    expect(fetchMock).toHaveBeenCalled();
  });

  // ── Uses custom apiUrl ──

  it('uses custom apiUrl when provided', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.custom', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://custom:7070'));

    await act(async () => {
      await result.current.getToken();
    });

    expect(fetchMock).toHaveBeenCalledWith(
      'http://custom:7070/api/auth/challenge',
      expect.any(Object),
    );
  });

  it('falls back to OPERATOR_API_URL when no apiUrl provided', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.default', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth());

    await act(async () => {
      await result.current.getToken();
    });

    expect(fetchMock).toHaveBeenCalledWith(
      'http://test-operator:9090/api/auth/challenge',
      expect.any(Object),
    );
  });

  it('reuses a valid shared token after hook remount', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.shared', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const first = renderHook(() => useOperatorAuth('http://test:9090'));
    await act(async () => {
      await first.result.current.getToken();
    });
    first.unmount();

    fetchMock.mockClear();

    const second = renderHook(() => useOperatorAuth('http://test:9090'));
    let token: string | null = null;
    await act(async () => {
      token = await second.result.current.getToken();
    });

    expect(token).toBe('v4.public.shared');
    expect(fetchMock).not.toHaveBeenCalled();
    expect(mockSignMessageAsync).toHaveBeenCalledTimes(1);
  });

  it('persists the session to sessionStorage for reuse after a refresh', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.persisted', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const first = renderHook(() => useOperatorAuth('http://test:9090'));
    await act(async () => {
      await first.result.current.getToken();
    });
    first.unmount();

    const storageKey = `tangle.operator_auth.${mockAddress.toLowerCase()}::http://test:9090`;
    const persisted = window.sessionStorage.getItem(storageKey);
    resetOperatorAuthStoreForTests();
    if (persisted) {
      window.sessionStorage.setItem(storageKey, persisted);
    }
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    const second = renderHook(() => useOperatorAuth('http://test:9090'));
    expect(second.result.current.getCachedToken()).toBe('v4.public.persisted');
    second.rerender();

    expect(second.result.current.isAuthenticated).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(mockSignMessageAsync).toHaveBeenCalledTimes(1);
  });

  it('getCachedToken returns null and does not sign when no cached session exists', () => {
    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    expect(result.current.getCachedToken()).toBeNull();
    expect(mockSignMessageAsync).not.toHaveBeenCalled();
  });

  it('deduplicates concurrent auth across hook instances for the same key', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Sign this', nonce: 'abc' },
      { token: 'v4.public.concurrent', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const first = renderHook(() => useOperatorAuth('http://test:9090'));
    const second = renderHook(() => useOperatorAuth('http://test:9090'));

    let tokens: Array<string | null> = [];
    await act(async () => {
      tokens = await Promise.all([
        first.result.current.getToken(),
        second.result.current.getToken(),
      ]);
    });

    expect(tokens).toEqual(['v4.public.concurrent', 'v4.public.concurrent']);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(mockSignMessageAsync).toHaveBeenCalledTimes(1);
  });

  it('does not share cached tokens across operator URLs', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = vi.fn();
    fetchMock.mockImplementation(async (url: string) => {
      if (url === 'http://first:9090/api/auth/challenge') {
        return { ok: true, json: async () => ({ message: 'Sign first', nonce: 'first' }) };
      }
      if (url === 'http://first:9090/api/auth/session') {
        return { ok: true, json: async () => ({ token: 'v4.public.first', expires_at: futureExpiry }) };
      }
      if (url === 'http://second:9090/api/auth/challenge') {
        return { ok: true, json: async () => ({ message: 'Sign second', nonce: 'second' }) };
      }
      if (url === 'http://second:9090/api/auth/session') {
        return { ok: true, json: async () => ({ token: 'v4.public.second', expires_at: futureExpiry }) };
      }
      return { ok: false, text: async () => 'Unknown endpoint' };
    });
    vi.stubGlobal('fetch', fetchMock);
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const first = renderHook(() => useOperatorAuth('http://first:9090'));
    const second = renderHook(() => useOperatorAuth('http://second:9090'));

    await act(async () => {
      await first.result.current.getToken();
    });

    fetchMock.mockClear();

    await act(async () => {
      await second.result.current.getToken();
    });

    expect(fetchMock).toHaveBeenCalledWith('http://second:9090/api/auth/challenge', expect.any(Object));
    expect(mockSignMessageAsync).toHaveBeenCalledTimes(2);
  });

  // ── Auth flow sends correct payloads ──

  it('sends address in challenge request and signature in session request', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    const fetchMock = mockFetchResponses(
      { message: 'Please sign this', nonce: 'nonce-123' },
      { token: 'v4.public.tok', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xmysignature');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    // Verify challenge request
    const challengeCall = fetchMock.mock.calls.find(
      (call) => (call[0] as string).includes('/api/auth/challenge'),
    );
    expect(challengeCall).toBeDefined();
    const challengeBody = JSON.parse(challengeCall![1].body);
    expect(challengeBody.address).toBe(mockAddress);

    // Verify session request
    const sessionCall = fetchMock.mock.calls.find(
      (call) => (call[0] as string).includes('/api/auth/session'),
    );
    expect(sessionCall).toBeDefined();
    const sessionBody = JSON.parse(sessionCall![1].body);
    expect(sessionBody.address).toBe(mockAddress);
    expect(sessionBody.signature).toBe('0xmysignature');
    expect(sessionBody.challenge).toBe('Please sign this');
    expect(sessionBody.nonce).toBe('nonce-123');
  });

  // ── isAuthenticating state ──

  it('isAuthenticating is false after auth completes', async () => {
    const futureExpiry = Math.floor(Date.now() / 1000) + 3600;
    mockFetchResponses(
      { message: 'Sign', nonce: 'n' },
      { token: 'tok', expires_at: futureExpiry },
    );
    mockSignMessageAsync.mockResolvedValue('0xsig');

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    expect(result.current.isAuthenticating).toBe(false);
  });

  it('isAuthenticating is false after auth error', async () => {
    mockFetchResponses(null, null);

    const { result } = renderHook(() => useOperatorAuth('http://test:9090'));

    await act(async () => {
      await result.current.getToken();
    });

    expect(result.current.isAuthenticating).toBe(false);
  });
});
