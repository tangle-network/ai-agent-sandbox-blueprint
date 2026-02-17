import { useCallback } from 'react';
import { getSession } from '~/lib/stores/session';
import { useSessionAuth } from './useSessionAuth';

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

interface UseAuthenticatedFetchOptions {
  sandboxId: string;
  apiUrl?: string;
}

/**
 * Returns a `fetch` wrapper that injects the Bearer session token.
 * If the token is expired or a 401 is received, triggers re-authentication.
 */
export function useAuthenticatedFetch({ sandboxId, apiUrl }: UseAuthenticatedFetchOptions) {
  const { authenticate } = useSessionAuth({ sandboxId, apiUrl });

  const authFetch = useCallback(
    async (url: string, init?: RequestInit): Promise<Response> => {
      let session = getSession(sandboxId);

      // If no session, authenticate first
      if (!session) {
        session = await authenticate();
        if (!session) {
          throw new Error('Authentication required');
        }
      }

      // Make request with token
      const headers = new Headers(init?.headers);
      headers.set('Authorization', `Bearer ${session.token}`);

      const res = await fetch(url, { ...init, headers });

      // If 401, try re-authenticating once
      if (res.status === 401) {
        session = await authenticate();
        if (!session) {
          throw new Error('Re-authentication failed');
        }

        const retryHeaders = new Headers(init?.headers);
        retryHeaders.set('Authorization', `Bearer ${session.token}`);
        return fetch(url, { ...init, headers: retryHeaders });
      }

      return res;
    },
    [sandboxId, authenticate],
  );

  return { authFetch };
}
