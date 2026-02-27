import { useEffect, useRef } from 'react';
import { toast } from 'sonner';
import { useOperatorAuth } from './useOperatorAuth';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { OPERATOR_API_URL, INSTANCE_OPERATOR_API_URL } from '~/lib/config';
import { fetchSandboxes, mergeApiResults, type ApiSandbox } from './sandboxHydrationLogic';

// Re-export for external consumers
export { fetchSandboxes, mergeApiResults, type ApiSandbox } from './sandboxHydrationLogic';

/**
 * Hydrate the local sandbox list from operator APIs on mount.
 *
 * Fetches from both the sandbox operator and (if configured) the instance
 * operator, then merges with local state. Shows a toast if the operator
 * API is unreachable.
 */
export function useSandboxHydration() {
  const { getToken: getSandboxToken } = useOperatorAuth(OPERATOR_API_URL);
  const { getToken: getInstanceToken } = useOperatorAuth(INSTANCE_OPERATOR_API_URL || undefined);
  const hydrated = useRef(false);

  useEffect(() => {
    if (hydrated.current) return;
    hydrated.current = true;

    const controller = new AbortController();
    const { signal } = controller;

    (async () => {
      const results: ApiSandbox[] = [];
      let hadError = false;

      // Fetch from sandbox operator
      try {
        const sandboxToken = await getSandboxToken();
        if (signal.aborted) return;
        if (sandboxToken) {
          const sandboxes = await fetchSandboxes(
            OPERATOR_API_URL,
            sandboxToken,
            import.meta.env.VITE_SANDBOX_BLUEPRINT_ID ?? '',
            import.meta.env.VITE_SANDBOX_SERVICE_ID ?? '',
            getSandboxToken,
            signal,
          );
          results.push(...sandboxes);
        }
      } catch (e) {
        if (signal.aborted) return;
        hadError = true;
        console.warn('Sandbox operator hydration failed:', e);
      }

      // Fetch from instance operator (if configured)
      if (INSTANCE_OPERATOR_API_URL) {
        try {
          const instanceToken = await getInstanceToken();
          if (signal.aborted) return;
          if (instanceToken) {
            const instances = await fetchSandboxes(
              INSTANCE_OPERATOR_API_URL,
              instanceToken,
              import.meta.env.VITE_INSTANCE_BLUEPRINT_ID ?? '',
              import.meta.env.VITE_INSTANCE_SERVICE_ID ?? '',
              getInstanceToken,
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

      // Surface error to user if sandbox operator is unreachable
      if (hadError && results.length === 0) {
        toast.error('Unable to reach operator API', {
          description: 'Sandbox status may be stale. Check that the operator is running.',
          duration: 6000,
        });
      }

      if (results.length === 0) return;

      const existing = sandboxListStore.get();
      const merged = mergeApiResults(results, existing);

      if (merged.length !== existing.length || merged.some((m, i) => m !== existing[i])) {
        sandboxListStore.set(merged);
      }
    })();

    return () => {
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- This is an on-mount effect.
    // The hydrated ref guarantees it runs at most once, so getToken deps are irrelevant.
  }, []);
}
