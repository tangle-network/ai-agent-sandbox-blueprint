import { useCallback, useState } from 'react';
import { useSignMessage } from 'wagmi';
import { getSession, setSession, removeSession, type SessionEntry } from '~/lib/stores/session';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ChallengeResponse {
  nonce: string;
  message: string;
  expires_at: number;
}

interface SessionResponse {
  token: string;
  address: string;
  expires_at: number;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

interface UseSessionAuthOptions {
  sandboxId: string;
  apiUrl?: string;
}

export function useSessionAuth({ sandboxId, apiUrl }: UseSessionAuthOptions) {
  const baseUrl = apiUrl ?? import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';
  const { signMessageAsync } = useSignMessage();

  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Get cached session
  const session = getSession(sandboxId);

  const authenticate = useCallback(async (): Promise<SessionEntry | null> => {
    setIsAuthenticating(true);
    setError(null);

    try {
      // Step 1: Request challenge
      const challengeRes = await fetch(`${baseUrl}/api/auth/challenge`, {
        method: 'POST',
      });
      if (!challengeRes.ok) {
        throw new Error(`Challenge request failed: HTTP ${challengeRes.status}`);
      }
      const challenge: ChallengeResponse = await challengeRes.json();

      // Step 2: Sign with wallet
      const signature = await signMessageAsync({ message: challenge.message });

      // Step 3: Exchange signature for session token
      const sessionRes = await fetch(`${baseUrl}/api/auth/session`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          nonce: challenge.nonce,
          signature,
        }),
      });
      if (!sessionRes.ok) {
        const body = await sessionRes.json().catch(() => ({}));
        throw new Error(body.error ?? `Session exchange failed: HTTP ${sessionRes.status}`);
      }
      const sessionData: SessionResponse = await sessionRes.json();

      // Step 4: Store session
      const entry: SessionEntry = {
        token: sessionData.token,
        address: sessionData.address,
        expiresAt: sessionData.expires_at,
        sandboxId,
      };
      setSession(entry);

      return entry;
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Authentication failed';
      setError(msg);
      return null;
    } finally {
      setIsAuthenticating(false);
    }
  }, [baseUrl, sandboxId, signMessageAsync]);

  const logout = useCallback(() => {
    removeSession(sandboxId);
  }, [sandboxId]);

  return {
    session,
    isAuthenticated: session !== null,
    isAuthenticating,
    error,
    authenticate,
    logout,
  };
}
