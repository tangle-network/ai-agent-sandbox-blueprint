import { useCallback, useSyncExternalStore } from 'react';
import { useAccount, useSignMessage } from 'wagmi';
import { OPERATOR_API_URL } from '~/lib/config';

interface OperatorSession {
  token: string;
  expiresAt: number;
}

interface OperatorAuthState {
  session: OperatorSession | null;
  inflight: Promise<string | null> | null;
  isAuthenticating: boolean;
  error: string | null;
}

const EMPTY_STATE: OperatorAuthState = {
  session: null,
  inflight: null,
  isAuthenticating: false,
  error: null,
};

const authRegistry = new Map<string, OperatorAuthState>();
const authListeners = new Map<string, Set<() => void>>();
const SESSION_STORAGE_PREFIX = 'tangle.operator_auth.';

function normalizeAddress(address: string): string {
  return address.toLowerCase();
}

function makeCacheKey(address: string, baseUrl: string): string {
  return `${normalizeAddress(address)}::${baseUrl}`;
}

function getPersistedSessionKey(key: string): string {
  return `${SESSION_STORAGE_PREFIX}${key}`;
}

function readPersistedSession(key: string): OperatorSession | null {
  if (typeof window === 'undefined' || !window.sessionStorage) return null;

  try {
    const raw = window.sessionStorage.getItem(getPersistedSessionKey(key));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<OperatorSession>;
    if (typeof parsed?.token !== 'string' || typeof parsed?.expiresAt !== 'number') {
      window.sessionStorage.removeItem(getPersistedSessionKey(key));
      return null;
    }
    const session: OperatorSession = { token: parsed.token, expiresAt: parsed.expiresAt };
    if (!isSessionValid(session)) {
      window.sessionStorage.removeItem(getPersistedSessionKey(key));
      return null;
    }
    return session;
  } catch {
    window.sessionStorage.removeItem(getPersistedSessionKey(key));
    return null;
  }
}

function persistSession(key: string, session: OperatorSession | null) {
  if (typeof window === 'undefined' || !window.sessionStorage) return;

  try {
    if (session && isSessionValid(session)) {
      window.sessionStorage.setItem(getPersistedSessionKey(key), JSON.stringify(session));
    } else {
      window.sessionStorage.removeItem(getPersistedSessionKey(key));
    }
  } catch {
    // Best-effort persistence only.
  }
}

function getState(key: string): OperatorAuthState {
  return authRegistry.get(key) ?? EMPTY_STATE;
}

function setState(key: string, next: OperatorAuthState) {
  authRegistry.set(key, next);
  persistSession(key, next.session);
  authListeners.get(key)?.forEach((listener) => listener());
}

function subscribeToKey(key: string, listener: () => void): () => void {
  const listeners = authListeners.get(key) ?? new Set<() => void>();
  listeners.add(listener);
  authListeners.set(key, listeners);
  return () => {
    const current = authListeners.get(key);
    if (!current) return;
    current.delete(listener);
    if (current.size === 0) {
      authListeners.delete(key);
    }
  };
}

function isSessionValid(session: OperatorSession | null): session is OperatorSession {
  if (!session) return false;
  // Consider expired 60s before actual expiry
  return session.expiresAt * 1000 > Date.now() + 60_000;
}

/**
 * Test-only helper to clear the shared auth registry between unit tests.
 */
export function resetOperatorAuthStoreForTests() {
  authRegistry.clear();
  authListeners.clear();
  if (typeof window !== 'undefined' && window.sessionStorage) {
    const keysToRemove: string[] = [];
    for (let i = 0; i < window.sessionStorage.length; i += 1) {
      const key = window.sessionStorage.key(i);
      if (key?.startsWith(SESSION_STORAGE_PREFIX)) keysToRemove.push(key);
    }
    keysToRemove.forEach((key) => window.sessionStorage.removeItem(key));
  }
}

/**
 * Hook to authenticate with the operator API via EIP-191 challenge/response.
 *
 * Flow:
 * 1. POST /api/auth/challenge → { message, nonce, expires_at }
 * 2. Sign message with wagmi wallet
 * 3. POST /api/auth/session → { token (PASETO), expires_at }
 *
 * The token is cached until 60s before expiry.
 */
export function useOperatorAuth(apiUrl?: string) {
  const baseUrl = apiUrl ?? OPERATOR_API_URL;
  const { address } = useAccount();
  const { signMessageAsync } = useSignMessage();
  const cacheKey = address ? makeCacheKey(address, baseUrl) : null;
  const subscribe = useCallback((listener: () => void) => {
    if (!cacheKey) return () => {};
    return subscribeToKey(cacheKey, listener);
  }, [cacheKey]);
  const getSnapshot = useCallback(() => {
    if (!cacheKey) return EMPTY_STATE;
    const current = getState(cacheKey);
    if (isSessionValid(current.session)) return current;
    const persisted = readPersistedSession(cacheKey);
    if (!persisted) return current;
    const restored = {
      ...current,
      session: persisted,
      error: null,
    };
    authRegistry.set(cacheKey, restored);
    return restored;
  }, [cacheKey]);
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  const getCachedToken = useCallback((): string | null => {
    if (!cacheKey) return null;
    const current = getState(cacheKey);
    if (isSessionValid(current.session)) return current.session.token;

    const persisted = readPersistedSession(cacheKey);
    if (!persisted) return null;

    setState(cacheKey, {
      ...current,
      session: persisted,
      inflight: null,
      isAuthenticating: false,
      error: null,
    });
    return persisted.token;
  }, [cacheKey]);

  const getToken = useCallback(async (forceRefresh = false): Promise<string | null> => {
    if (!address || !cacheKey) return null;

    const current = getState(cacheKey);
    if (!forceRefresh && isSessionValid(current.session)) return current.session.token;
    if (!forceRefresh) {
      const persisted = readPersistedSession(cacheKey);
      if (persisted) {
        setState(cacheKey, {
          ...current,
          session: persisted,
          inflight: null,
          isAuthenticating: false,
          error: null,
        });
        return persisted.token;
      }
    }
    if (current.inflight) return current.inflight;
    if (forceRefresh) {
      setState(cacheKey, { ...current, session: null, error: null });
    }

    const promise = (async () => {
      try {
        // Step 1: Get challenge
        const challengeRes = await fetch(`${baseUrl}/api/auth/challenge`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ address }),
        });
        if (!challengeRes.ok) {
          throw new Error(`Challenge failed: ${await challengeRes.text()}`);
        }
        const { message, nonce } = await challengeRes.json();

        // Step 2: Sign with wallet
        const signature = await signMessageAsync({ message });

        // Step 3: Create session
        const sessionRes = await fetch(`${baseUrl}/api/auth/session`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ address, signature, challenge: message, nonce }),
        });
        if (!sessionRes.ok) {
          throw new Error(`Session creation failed: ${await sessionRes.text()}`);
        }
        const { token, expires_at } = await sessionRes.json();

        setState(cacheKey, {
          session: { token, expiresAt: expires_at },
          inflight: null,
          isAuthenticating: false,
          error: null,
        });
        return token;
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Auth failed';
        setState(cacheKey, {
          ...getState(cacheKey),
          session: null,
          inflight: null,
          isAuthenticating: false,
          error: msg,
        });
        return null;
      }
    })();

    setState(cacheKey, {
      ...getState(cacheKey),
      inflight: promise,
      isAuthenticating: true,
      error: null,
    });

    return promise;
  }, [address, baseUrl, cacheKey, signMessageAsync]);

  return {
    /** Get a valid cached PASETO token without triggering wallet signing. */
    getCachedToken,
    /** Get a valid PASETO token, authenticating if needed. */
    getToken,
    /** Whether we have a valid cached token. */
    isAuthenticated: isSessionValid(state.session),
    /** Whether an auth request is in-flight. */
    isAuthenticating: state.isAuthenticating,
    /** Last error message, if any. */
    error: state.error,
  };
}
