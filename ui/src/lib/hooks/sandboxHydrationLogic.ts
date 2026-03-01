/**
 * Pure functions for sandbox hydration.
 *
 * Separated from the hook so they can be tested without pulling in
 * React, wagmi, or other heavy runtime dependencies.
 */

import type { LocalSandbox } from '~/lib/stores/sandboxes';

export interface ApiSandbox {
  id: string;
  name?: string;
  sidecar_url: string;
  state: string;
  image?: string;
  agent_identifier?: string;
  cpu_cores: number;
  memory_mb: number;
  disk_gb?: number;
  created_at: number;
  last_activity_at: number;
  ssh_port?: number;
  tee_deployment_id?: string;
}

export async function fetchSandboxes(
  baseUrl: string,
  token: string,
  blueprintId: string,
  serviceId: string,
  getToken?: (forceRefresh: boolean) => Promise<string | null>,
  signal?: AbortSignal,
): Promise<ApiSandbox[]> {
  const url = `${baseUrl}/api/sandboxes`;
  let res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
    signal,
  });

  // Auto-retry once on 401 (expired PASETO token)
  if (res.status === 401 && getToken) {
    const freshToken = await getToken(true);
    if (freshToken) {
      res = await fetch(url, {
        headers: { Authorization: `Bearer ${freshToken}` },
        signal,
      });
    }
  }

  if (!res.ok) return [];
  const data = await res.json();
  return data.sandboxes ?? [];
}

/** Merge API results with local sandbox state. */
export function mergeApiResults(apiResults: ApiSandbox[], existing: LocalSandbox[]): LocalSandbox[] {
  const existingIds = new Set(existing.map((s) => s.id));

  const newSandboxes: LocalSandbox[] = apiResults
    .filter((s) => !existingIds.has(s.id))
    .map((s) => ({
      id: s.id,
      name: s.name || s.id.replace('sandbox-', '').slice(0, 8),
      image: s.image || '',
      cpuCores: s.cpu_cores,
      memoryMb: s.memory_mb,
      diskGb: s.disk_gb || 0,
      createdAt: s.created_at * 1000,
      blueprintId: '',
      serviceId: '',
      sidecarUrl: s.sidecar_url,
      agentIdentifier: s.agent_identifier || undefined,
      teeEnabled: !!s.tee_deployment_id,
      status: (s.state === 'running' ? 'running' : 'stopped') as LocalSandbox['status'],
    }));

  const apiStatusMap = new Map(apiResults.map((s) => [s.id, s]));
  const updated = existing.map((local) => {
    const api = apiStatusMap.get(local.id);
    if (!api) return local;
    return {
      ...local,
      sidecarUrl: api.sidecar_url || local.sidecarUrl,
      image: api.image || local.image,
      agentIdentifier: api.agent_identifier || local.agentIdentifier,
      teeEnabled: local.teeEnabled || !!api.tee_deployment_id,
      status: (api.state === 'running' ? 'running' : 'stopped') as LocalSandbox['status'],
    };
  });

  return [...newSandboxes, ...updated];
}
