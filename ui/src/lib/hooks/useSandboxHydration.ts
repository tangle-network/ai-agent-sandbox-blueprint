import { useEffect, useRef } from 'react';
import { useOperatorAuth } from './useOperatorAuth';
import { sandboxListStore, type LocalSandbox } from '~/lib/stores/sandboxes';

const OPERATOR_API_URL = import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';

/**
 * Hydrate the local sandbox list from the operator API on mount.
 *
 * Merges API results with existing local state — API sandboxes that aren't
 * in the local store are added, and running/stopped status is updated from
 * the operator's ground truth.
 */
export function useSandboxHydration() {
  const { getToken } = useOperatorAuth();
  const hydrated = useRef(false);

  useEffect(() => {
    if (hydrated.current) return;
    hydrated.current = true;

    (async () => {
      try {
        const token = await getToken();
        if (!token) return;

        const res = await fetch(`${OPERATOR_API_URL}/api/sandboxes`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (!res.ok) return;

        const data = await res.json();
        const apiSandboxes: Array<{
          id: string;
          sidecar_url: string;
          state: string;
          cpu_cores: number;
          memory_mb: number;
          created_at: number;
          last_activity_at: number;
        }> = data.sandboxes ?? [];

        const existing = sandboxListStore.get();
        const existingIds = new Set(existing.map((s) => s.id));

        // Add new sandboxes from API that aren't in local store
        const newSandboxes: LocalSandbox[] = apiSandboxes
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
        const apiStatusMap = new Map(apiSandboxes.map((s) => [s.id, s]));
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
      } catch {
        // Silently fail — hydration is best-effort
      }
    })();
  }, [getToken]);
}
