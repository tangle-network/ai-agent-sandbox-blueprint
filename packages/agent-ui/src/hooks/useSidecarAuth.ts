import { useState, useCallback, useEffect, useRef } from 'react';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface UseSidecarAuthOptions {
  /** Scoping key for token storage (e.g. botId, sandboxId). */
  resourceId: string;
  /** Base URL of the sidecar or operator API. */
  apiUrl: string;
  /**
   * Sign a plaintext message and return the hex signature.
   * Consuming apps wire this to their wallet library (e.g. wagmi's signMessageAsync).
   */
  signMessage: (message: string) => Promise<string>;
}

export interface SidecarAuth {
  token: string | null;
  isAuthenticated: boolean;
  isAuthenticating: boolean;
  authenticate: () => Promise<string | null>;
  clearCachedToken: () => void;
  error: string | null;
}

// ---------------------------------------------------------------------------
// localStorage helpers
// ---------------------------------------------------------------------------

function storageKey(resourceId: string, apiUrl: string): string {
  return `sidecar_session_${resourceId}__${apiUrl}`;
}

function loadSession(resourceId: string, apiUrl: string): { token: string; expiresAt: number } | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = localStorage.getItem(storageKey(resourceId, apiUrl));
    if (!raw) return null;
    const data = JSON.parse(raw) as { token: string; expiresAt: number };
    // Discard if within 60s of expiry
    if (data.expiresAt * 1000 - Date.now() < 60_000) {
      localStorage.removeItem(storageKey(resourceId, apiUrl));
      return null;
    }
    return data;
  } catch {
    return null;
  }
}

function saveSession(resourceId: string, apiUrl: string, token: string, expiresAt: number) {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(storageKey(resourceId, apiUrl), JSON.stringify({ token, expiresAt }));
  } catch {
    // storage full — ignore
  }
}

function clearSession(resourceId: string, apiUrl: string) {
  if (typeof window === 'undefined') return;
  localStorage.removeItem(storageKey(resourceId, apiUrl));
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Generic sidecar PASETO challenge/response auth.
 *
 * Flow:
 * 1. POST /api/auth/challenge  → { nonce, message, expires_at }
 * 2. signMessage(message)       → signature (provided by consuming app)
 * 3. POST /api/auth/session     → { token, address, expires_at }
 *
 * Tokens are cached in localStorage and auto-refreshed 5 minutes before expiry.
 *
 * @deprecated Browser applications should prefer operator-proxied access
 * instead of direct sidecar authentication. This hook remains for backwards
 * compatibility with existing direct integrations.
 */
export function useSidecarAuth({ resourceId, apiUrl, signMessage }: UseSidecarAuthOptions): SidecarAuth {
  const cached = loadSession(resourceId, apiUrl);
  const [token, setToken] = useState<string | null>(cached?.token ?? null);
  const [expiresAt, setExpiresAt] = useState<number>(cached?.expiresAt ?? 0);
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout>>();

  const clearCachedToken = useCallback(() => {
    setToken(null);
    setExpiresAt(0);
    clearSession(resourceId, apiUrl);
  }, [resourceId, apiUrl]);

  const authenticate = useCallback(async (): Promise<string | null> => {
    if (!apiUrl) return null;
    setIsAuthenticating(true);
    setError(null);

    try {
      // Step 1: Get challenge
      const challengeRes = await fetch(`${apiUrl}/api/auth/challenge`, {
        method: 'POST',
      });
      if (!challengeRes.ok) {
        throw new Error(`Challenge failed: ${challengeRes.status}`);
      }
      const { nonce, message } = await challengeRes.json();

      // Step 2: Sign with wallet (injected)
      const signature = await signMessage(message);

      // Step 3: Exchange for session token
      const sessionRes = await fetch(`${apiUrl}/api/auth/session`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ nonce, signature }),
      });
      if (!sessionRes.ok) {
        const text = await sessionRes.text();
        throw new Error(text || `Session exchange failed: ${sessionRes.status}`);
      }

      const { token: newToken, expires_at } = await sessionRes.json();
      setToken(newToken);
      setExpiresAt(expires_at);
      saveSession(resourceId, apiUrl, newToken, expires_at);
      return newToken;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Authentication failed');
      clearCachedToken();
      return null;
    } finally {
      setIsAuthenticating(false);
    }
  }, [resourceId, apiUrl, signMessage, clearCachedToken]);

  // Auto-refresh token 5 minutes before expiry
  useEffect(() => {
    if (refreshTimerRef.current) {
      clearTimeout(refreshTimerRef.current);
    }

    if (!token || !expiresAt) return;

    const msUntilRefresh = (expiresAt - 300) * 1000 - Date.now();
    if (msUntilRefresh <= 0) {
      clearCachedToken();
      return;
    }

    refreshTimerRef.current = setTimeout(() => {
      authenticate().catch(() => {
        clearCachedToken();
      });
    }, msUntilRefresh);

    return () => {
      if (refreshTimerRef.current) {
        clearTimeout(refreshTimerRef.current);
      }
    };
  }, [token, expiresAt, authenticate, clearCachedToken]);

  return {
    token,
    isAuthenticated: token !== null,
    isAuthenticating,
    authenticate,
    clearCachedToken,
    error,
  };
}
