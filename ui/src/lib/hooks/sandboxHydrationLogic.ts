/**
 * Pure functions for sandbox hydration.
 *
 * Separated from the hook so they can be tested without pulling in
 * React, wagmi, or other heavy runtime dependencies.
 */

import {
  type LocalSandbox,
  getSandboxRouteKey,
  normalizeSandbox,
} from '~/lib/stores/sandboxes';

const DRAFT_TX_GRACE_MS = 15 * 60 * 1000;
const DRAFT_MATCH_WINDOW_MS = 10 * 60 * 1000;

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

export interface ApiProvision {
  call_id: number;
  phase: string;
  sandbox_id?: string | null;
  sidecar_url?: string | null;
  message?: string | null;
}

export async function fetchSandboxes(
  baseUrl: string,
  token: string,
  blueprintId: string,
  serviceId: string,
  getToken?: (forceRefresh: boolean) => Promise<string | null>,
  signal?: AbortSignal,
  opts: { throwOnError?: boolean } = {},
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

  if (!res.ok) {
    if (opts.throwOnError) {
      const body = typeof res.text === 'function' ? await res.text().catch(() => '') : '';
      throw new Error(body || `Sandbox list request failed with ${res.status}`);
    }
    return [];
  }
  const data = await res.json();
  return data.sandboxes ?? [];
}

/** Merge API results with local sandbox state. */
export function mergeApiResults(apiResults: ApiSandbox[], existing: LocalSandbox[]): LocalSandbox[] {
  return reconcileSandboxes(existing, apiResults, new Map(), {
    pruneUnverifiedDrafts: false,
    pruneMissingCanonical: false,
  });
}

function statusFromApi(state: string): LocalSandbox['status'] {
  return state === 'running' ? 'running' : 'stopped';
}

function sandboxFromApi(api: ApiSandbox): LocalSandbox {
  return normalizeSandbox({
    localId: `canonical:${api.id}`,
    sandboxId: api.id,
    name: api.name || api.id.replace('sandbox-', '').slice(0, 8),
    image: api.image || '',
    cpuCores: api.cpu_cores,
    memoryMb: api.memory_mb,
    diskGb: api.disk_gb || 0,
    createdAt: api.created_at * 1000,
    blueprintId: '',
    serviceId: '',
    sidecarUrl: api.sidecar_url,
    agentIdentifier: api.agent_identifier || undefined,
    teeEnabled: !!api.tee_deployment_id,
    status: statusFromApi(api.state),
  });
}

function hasRecentPendingTx(sandbox: LocalSandbox): boolean {
  if (!sandbox.txHash || sandbox.status !== 'creating') return false;
  return Date.now() - sandbox.createdAt <= DRAFT_TX_GRACE_MS;
}

function normalizeAgentIdentifier(value: string | undefined): string {
  return (value || '').trim();
}

function matchesDraftFingerprint(local: LocalSandbox, api: ApiSandbox): boolean {
  if ((api.name || '') !== local.name) return false;
  if ((api.image || '') !== local.image) return false;
  if (api.cpu_cores !== local.cpuCores) return false;
  if (api.memory_mb !== local.memoryMb) return false;
  if ((api.disk_gb || 0) !== local.diskGb) return false;
  if (normalizeAgentIdentifier(api.agent_identifier) !== normalizeAgentIdentifier(local.agentIdentifier)) {
    return false;
  }

  return Math.abs((api.created_at * 1000) - local.createdAt) <= DRAFT_MATCH_WINDOW_MS;
}

function getUniqueDraftMatch(
  local: LocalSandbox,
  apiResults: ApiSandbox[],
  matchedApiIds: Set<string>,
): ApiSandbox | null {
  const candidates = apiResults.filter((api) =>
    !matchedApiIds.has(api.id) && matchesDraftFingerprint(local, api),
  );

  if (candidates.length === 1) return candidates[0];
  if (candidates.length > 1) {
    console.warn('Ambiguous sandbox draft match; leaving draft unchanged', {
      draftLocalId: local.localId,
      candidateIds: candidates.map((candidate) => candidate.id),
    });
  }
  return null;
}

export function reconcileSandboxes(
  existing: LocalSandbox[],
  apiResults: ApiSandbox[],
  provisionsByCallId: Map<number, ApiProvision | null>,
  opts: { pruneUnverifiedDrafts: boolean; pruneMissingCanonical: boolean },
): LocalSandbox[] {
  const apiById = new Map(apiResults.map((sandbox) => [sandbox.id, sandbox]));
  const matchedApiIds = new Set<string>();
  const reconciled: LocalSandbox[] = [];

  for (const local of existing.map(normalizeSandbox)) {
    let next = local;
    const provision = next.callId != null ? provisionsByCallId.get(next.callId) : undefined;

    if (!next.sandboxId && provision?.phase === 'ready' && provision.sandbox_id) {
      next = normalizeSandbox({
        ...next,
        sandboxId: provision.sandbox_id ?? undefined,
        sidecarUrl: provision.sidecar_url || next.sidecarUrl,
        status: 'running',
      });
    } else if (!next.sandboxId && provision?.phase === 'failed') {
      next = normalizeSandbox({ ...next, status: 'error' });
    }

    const api = next.sandboxId ? apiById.get(next.sandboxId) : undefined;
    if (api) {
      matchedApiIds.add(api.id);
      reconciled.push(normalizeSandbox({
        ...next,
        sandboxId: api.id,
        sidecarUrl: api.sidecar_url || next.sidecarUrl,
        image: api.image || next.image,
        agentIdentifier: api.agent_identifier || next.agentIdentifier,
        teeEnabled: next.teeEnabled || !!api.tee_deployment_id,
        status: statusFromApi(api.state),
      }));
      continue;
    }

    if (next.sandboxId && opts.pruneMissingCanonical) {
      continue;
    }

    if (!next.sandboxId) {
      const inferredApi = getUniqueDraftMatch(next, apiResults, matchedApiIds);
      if (inferredApi) {
        matchedApiIds.add(inferredApi.id);
        reconciled.push(normalizeSandbox({
          ...next,
          sandboxId: inferredApi.id,
          sidecarUrl: inferredApi.sidecar_url || next.sidecarUrl,
          image: inferredApi.image || next.image,
          agentIdentifier: inferredApi.agent_identifier || next.agentIdentifier,
          teeEnabled: next.teeEnabled || !!inferredApi.tee_deployment_id,
          status: statusFromApi(inferredApi.state),
        }));
        continue;
      }

      if (next.callId != null) {
        if (provision === null && opts.pruneUnverifiedDrafts) {
          continue;
        }
        reconciled.push(next);
        continue;
      }
      if (hasRecentPendingTx(next)) {
        reconciled.push(next);
        continue;
      }
      if (opts.pruneUnverifiedDrafts) {
        continue;
      }
    }

    reconciled.push(next);
  }

  for (const api of apiResults) {
    if (matchedApiIds.has(api.id)) continue;
    reconciled.push(sandboxFromApi(api));
  }

  const seenRouteKeys = new Set<string>();
  return reconciled.filter((sandbox) => {
    const routeKey = getSandboxRouteKey(sandbox);
    if (seenRouteKeys.has(routeKey)) return false;
    seenRouteKeys.add(routeKey);
    return true;
  });
}
