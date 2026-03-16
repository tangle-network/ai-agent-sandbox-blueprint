import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { useOperatorAuth } from './useOperatorAuth';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { OPERATOR_API_URL, INSTANCE_OPERATOR_API_URL } from '~/lib/config';
import {
  fetchSandboxes,
  reconcileSandboxes,
  type ApiProvision,
  type ApiSandbox,
} from './sandboxHydrationLogic';

// Re-export for external consumers
export {
  fetchSandboxes,
  reconcileSandboxes,
  type ApiProvision,
  type ApiSandbox,
} from './sandboxHydrationLogic';

interface RefreshOpts {
  interactive?: boolean;
}

export interface SandboxHydrationState {
  refresh: (opts?: RefreshOpts) => Promise<void>;
  isHydrating: boolean;
  authRequired: boolean;
  lastError: string | null;
}

/**
 * Hydrate the local sandbox list from operator APIs on mount.
 *
 * Fetches from both the sandbox operator and (if configured) the instance
 * operator, then merges with local state. Shows a toast if the operator
 * API is unreachable.
 */
export function useSandboxHydration() {
  const { getToken: getSandboxToken, getCachedToken: getCachedSandboxToken } = useOperatorAuth(OPERATOR_API_URL);
  const { getToken: getInstanceToken, getCachedToken: getCachedInstanceToken } = useOperatorAuth(INSTANCE_OPERATOR_API_URL || undefined);
  const controllerRef = useRef<AbortController | null>(null);
  const [isHydrating, setIsHydrating] = useState(false);
  const [authRequired, setAuthRequired] = useState(false);
  const [lastError, setLastError] = useState<string | null>(null);

  const refresh = useCallback(async ({ interactive = false }: RefreshOpts = {}) => {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const { signal } = controller;

    setIsHydrating(true);
    setLastError(null);
    setAuthRequired(false);

    try {
      const fetchProvisionStatuses = async () => {
        const existing = sandboxListStore.get();
        const drafts = existing.filter((sandbox) => !sandbox.sandboxId && sandbox.callId != null);
        const provisions = new Map<number, ApiProvision | null>();
        if (drafts.length === 0) return provisions;

        await Promise.all(
          drafts.map(async (sandbox) => {
            try {
              const res = await fetch(`${OPERATOR_API_URL}/api/provisions/${sandbox.callId}`, { signal });
              if (res.status === 404) {
                provisions.set(sandbox.callId!, null);
                return;
              }
              if (!res.ok) return;
              const body = await res.json();
              provisions.set(sandbox.callId!, body);
            } catch {
              // Keep the optimistic local state if we cannot verify the provision record.
            }
          }),
        );

        return provisions;
      };

      const results: ApiSandbox[] = [];
      let hadError = false;
      let sandboxFetchSucceeded = false;
      let sandboxAuthRequired = false;
      const provisionResults = await fetchProvisionStatuses();

      // Fetch from sandbox operator
      try {
        const sandboxToken = interactive ? await getSandboxToken() : getCachedSandboxToken();
        if (signal.aborted) return;
        if (sandboxToken) {
          const sandboxes = await fetchSandboxes(
            OPERATOR_API_URL,
            sandboxToken,
            import.meta.env.VITE_SANDBOX_BLUEPRINT_ID ?? '',
            import.meta.env.VITE_SANDBOX_SERVICE_ID ?? '',
            interactive ? getSandboxToken : undefined,
            signal,
            { throwOnError: interactive },
          );
          results.push(...sandboxes);
          sandboxFetchSucceeded = true;
        } else {
          sandboxAuthRequired = true;
          if (interactive) {
            const message = 'Operator authentication was cancelled or failed';
            setLastError(message);
            toast.error(message, {
              description: 'Sign the wallet challenge to refresh sandbox state from the operator.',
              duration: 6000,
            });
          }
        }
      } catch (e) {
        if (signal.aborted) return;
        hadError = true;
        console.warn('Sandbox operator hydration failed:', e);
        if (interactive) {
          const message = e instanceof Error ? e.message : 'Unable to refresh sandboxes';
          setLastError(message);
          toast.error('Unable to refresh sandboxes', {
            description: message,
            duration: 6000,
          });
        }
      }

      // Fetch from instance operator (if configured)
      if (INSTANCE_OPERATOR_API_URL) {
        try {
          const instanceToken = interactive ? await getInstanceToken() : getCachedInstanceToken();
          if (signal.aborted) return;
          if (instanceToken) {
            const instances = await fetchSandboxes(
              INSTANCE_OPERATOR_API_URL,
              instanceToken,
              import.meta.env.VITE_INSTANCE_BLUEPRINT_ID ?? '',
              import.meta.env.VITE_INSTANCE_SERVICE_ID ?? '',
              interactive ? getInstanceToken : undefined,
              signal,
            );
            results.push(...instances);
          }
        } catch (e) {
          if (signal.aborted) return;
          console.warn('Instance operator hydration failed:', e);
        }
      }

      if (signal.aborted) return;

      setAuthRequired(sandboxAuthRequired);

      // Surface error to user if sandbox operator is unreachable
      if (hadError && results.length === 0) {
        if (!interactive) {
          setLastError('Unable to reach operator API');
          toast.error('Unable to reach operator API', {
            description: 'Sandbox status may be stale. Check that the operator is running.',
            duration: 6000,
          });
        }
        return;
      }

      const existing = sandboxListStore.get();
      const merged = reconcileSandboxes(existing, results, provisionResults, {
        pruneUnverifiedDrafts: sandboxFetchSucceeded,
        pruneMissingCanonical: sandboxFetchSucceeded,
      });

      if (merged.length !== existing.length || merged.some((m, i) => m !== existing[i])) {
        sandboxListStore.set(merged);
      }
    } finally {
      if (controllerRef.current === controller) {
        controllerRef.current = null;
      }
      if (!signal.aborted) {
        setIsHydrating(false);
      }
    }
  }, [
    getCachedInstanceToken,
    getCachedSandboxToken,
    getInstanceToken,
    getSandboxToken,
  ]);

  useEffect(() => {
    void refresh({ interactive: false });

    const handleVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      void refresh({ interactive: false });
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      controllerRef.current?.abort();
    };
  }, [refresh]);

  return {
    refresh,
    isHydrating,
    authRequired,
    lastError,
  };
}
