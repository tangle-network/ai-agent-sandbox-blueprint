import { useCallback, useEffect, useRef, useState } from 'react';
import { useAccount, useSignMessage } from 'wagmi';

const OPERATOR_API_URL = import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';

interface OperatorSession {
  token: string;
  expiresAt: number;
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
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sessionRef = useRef<OperatorSession | null>(null);
  const inflightRef = useRef<Promise<string | null> | null>(null);

  // Clear stale session when wallet address changes
  useEffect(() => {
    sessionRef.current = null;
  }, [address]);

  const isValid = useCallback(() => {
    if (!sessionRef.current) return false;
    // Consider expired 60s before actual expiry
    return sessionRef.current.expiresAt * 1000 > Date.now() + 60_000;
  }, []);

  const getToken = useCallback(async (forceRefresh = false): Promise<string | null> => {
    if (!forceRefresh && isValid()) return sessionRef.current!.token;
    // Deduplicate concurrent calls
    if (inflightRef.current && !forceRefresh) return inflightRef.current;
    if (forceRefresh) sessionRef.current = null;
    if (!address) return null;

    const promise = (async () => {
      setIsAuthenticating(true);
      setError(null);

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

        sessionRef.current = { token, expiresAt: expires_at };
        return token;
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Auth failed';
        setError(msg);
        return null;
      } finally {
        setIsAuthenticating(false);
      }
    })();

    inflightRef.current = promise;
    try {
      return await promise;
    } finally {
      inflightRef.current = null;
    }
  }, [address, baseUrl, isValid, signMessageAsync]);

  return {
    /** Get a valid PASETO token, authenticating if needed. */
    getToken,
    /** Whether we have a valid cached token. */
    isAuthenticated: isValid(),
    /** Whether an auth request is in-flight. */
    isAuthenticating,
    /** Last error message, if any. */
    error,
  };
}
