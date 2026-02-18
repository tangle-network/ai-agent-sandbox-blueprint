import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { Session } from '~/types/sidecar';

// ---------------------------------------------------------------------------
// Fetch helper
// ---------------------------------------------------------------------------

async function sessionFetch<T>(
  apiUrl: string,
  token: string,
  path: string,
  opts?: RequestInit,
): Promise<T> {
  const res = await fetch(`${apiUrl}${path}`, {
    ...opts,
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
      ...(opts?.headers ?? {}),
    },
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `API error: ${res.status}`);
  }
  return res.json();
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/** List all sessions for a sidecar. */
export function useSessions(apiUrl: string, token: string | null) {
  return useQuery<Session[]>({
    queryKey: ['sessions', apiUrl, token],
    queryFn: () => sessionFetch<Session[]>(apiUrl, token!, '/session/sessions'),
    enabled: !!token,
    staleTime: 30_000,
  });
}

/** Create a new session. */
export function useCreateSession(apiUrl: string, token: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (title: string) => {
      return sessionFetch<Session>(apiUrl, token!, '/session/sessions', {
        method: 'POST',
        body: JSON.stringify({ title, backend: { type: 'opencode' } }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sessions', apiUrl, token] });
    },
  });
}

/** Delete a session by ID. */
export function useDeleteSession(apiUrl: string, token: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (sessionId: string) => {
      return sessionFetch<unknown>(apiUrl, token!, `/session/sessions/${encodeURIComponent(sessionId)}`, {
        method: 'DELETE',
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sessions', apiUrl, token] });
    },
  });
}

/** Rename a session. */
export function useRenameSession(apiUrl: string, token: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ sessionId, title }: { sessionId: string; title: string }) => {
      return sessionFetch<Session>(apiUrl, token!, `/session/sessions/${encodeURIComponent(sessionId)}`, {
        method: 'PATCH',
        body: JSON.stringify({ title }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sessions', apiUrl, token] });
    },
  });
}
