import { useEffect, useRef } from 'react';
import { toast } from 'sonner';
import { useOperatorAuth } from './useOperatorAuth';
import { sandboxListStore, type LocalSandbox } from '~/lib/stores/sandboxes';

const OPERATOR_API_URL = import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';
const INSTANCE_OPERATOR_API_URL = import.meta.env.VITE_INSTANCE_OPERATOR_API_URL ?? '';

interface ApiSandbox {
  id: string;
  sidecar_url: string;
  state: string;
  cpu_cores: number;
  memory_mb: number;
  created_at: number;
  last_activity_at: number;
}

async function fetchSandboxes(
  baseUrl: string,
  token: string,
  blueprintId: string,
  serviceId: string,
  getToken?: (forceRefresh: boolean) => Promise<string | null>,
): Promise<ApiSandbox[]> {
  const url = `${baseUrl}/api/sandboxes`;
  let res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
  });

  // Auto-retry once on 401 (expired PASETO token)
  if (res.status === 401 && getToken) {
    const freshToken = await getToken(true);
    if (freshToken) {
      res = await fetch(url, {
        headers: { Authorization: `Bearer ${freshToken}` },
      });
    }
  }

  if (!res.ok) return [];
  const data = await res.json();
  return data.sandboxes ?? [];
}

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

    (async () => {
      const results: ApiSandbox[] = [];
      let hadError = false;

      // Fetch from sandbox operator
      try {
        const sandboxToken = await getSandboxToken();
        if (sandboxToken) {
          const sandboxes = await fetchSandboxes(
            OPERATOR_API_URL,
            sandboxToken,
            import.meta.env.VITE_SANDBOX_BLUEPRINT_ID ?? '',
            import.meta.env.VITE_SANDBOX_SERVICE_ID ?? '',
            getSandboxToken,
          );
          results.push(...sandboxes);
        }
      } catch (e) {
        hadError = true;
        console.warn('Sandbox operator hydration failed:', e);
      }

      // Fetch from instance operator (if configured)
      if (INSTANCE_OPERATOR_API_URL) {
        try {
          const instanceToken = await getInstanceToken();
          if (instanceToken) {
            const instances = await fetchSandboxes(
              INSTANCE_OPERATOR_API_URL,
              instanceToken,
              import.meta.env.VITE_INSTANCE_BLUEPRINT_ID ?? '',
              import.meta.env.VITE_INSTANCE_SERVICE_ID ?? '',
              getInstanceToken,
            );
            results.push(...instances);
          }
        } catch (e) {
          console.warn('Instance operator hydration failed:', e);
        }
      }

      // Surface error to user if sandbox operator is unreachable
      if (hadError && results.length === 0) {
        toast.error('Unable to reach operator API', {
          description: 'Sandbox status may be stale. Check that the operator is running.',
          duration: 6000,
        });
      }

      if (results.length === 0) return;

      const existing = sandboxListStore.get();
      const existingIds = new Set(existing.map((s) => s.id));

      // Add new sandboxes from API that aren't in local store
      const newSandboxes: LocalSandbox[] = results
        .filter((s) => !existingIds.has(s.id))
        .map((s) => ({
          id: s.id,
          name: s.id.replace('sandbox-', '').slice(0, 8),
          image: '',
          cpuCores: s.cpu_cores,
          memoryMb: s.memory_mb,
          diskGb: 0,
          createdAt: s.created_at * 1000,
          blueprintId: import.meta.env.VITE_SANDBOX_BLUEPRINT_ID ?? '',
          serviceId: import.meta.env.VITE_SANDBOX_SERVICE_ID ?? '',
          sidecarUrl: s.sidecar_url,
          status: s.state === 'running' ? 'running' : 'stopped',
        }));

      // Update status of existing sandboxes from API ground truth
      const apiStatusMap = new Map(results.map((s) => [s.id, s]));
      const updated = existing.map((local) => {
        const api = apiStatusMap.get(local.id);
        if (!api) return local;
        return {
          ...local,
          sidecarUrl: api.sidecar_url || local.sidecarUrl,
          status: (api.state === 'running' ? 'running' : 'stopped') as LocalSandbox['status'],
        };
      });

      if (newSandboxes.length > 0 || updated.some((u, i) => u !== existing[i])) {
        sandboxListStore.set([...newSandboxes, ...updated]);
      }
    })();
  }, [getSandboxToken, getInstanceToken]);
}
