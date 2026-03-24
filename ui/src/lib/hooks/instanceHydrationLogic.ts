import type { ApiSandbox } from './sandboxHydrationLogic';
import type { LocalInstance } from '~/lib/stores/instances';

const DRAFT_MATCH_WINDOW_MS = 10 * 60 * 1000;

function normalizeAgentIdentifier(value: string | undefined): string {
  return (value || '').trim();
}

function statusFromApi(state: string): LocalInstance['status'] {
  return state === 'running' ? 'running' : 'stopped';
}

function matchesDraftFingerprint(local: LocalInstance, api: ApiSandbox): boolean {
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
  local: LocalInstance,
  apiResults: ApiSandbox[],
  matchedApiIds: Set<string>,
): ApiSandbox | null {
  const candidates = apiResults.filter((api) =>
    !matchedApiIds.has(api.id) && matchesDraftFingerprint(local, api),
  );

  if (candidates.length === 1) return candidates[0];
  return null;
}

function instanceFromApi(api: ApiSandbox): LocalInstance {
  return {
    id: api.id,
    sandboxId: api.id,
    name: api.name || api.id.replace('sandbox-', '').slice(0, 8),
    image: api.image || '',
    cpuCores: api.cpu_cores,
    memoryMb: api.memory_mb,
    diskGb: api.disk_gb || 0,
    createdAt: api.created_at * 1000,
    blueprintId: '',
    serviceId: api.service_id != null ? String(api.service_id) : '',
    operator: api.managing_operator || undefined,
    sidecarUrl: api.sidecar_url,
    agentIdentifier: api.agent_identifier || undefined,
    teeEnabled: !!api.tee_deployment_id,
    credentialsAvailable: api.credentials_available ?? undefined,
    sshPort: api.ssh_port || undefined,
    status: statusFromApi(api.state),
  };
}

export function reconcileInstances(
  existing: LocalInstance[],
  apiResults: ApiSandbox[],
  serviceIdsByRequestId: Map<number, string>,
): LocalInstance[] {
  const apiById = new Map(apiResults.map((sandbox) => [sandbox.id, sandbox]));
  const matchedApiIds = new Set<string>();
  const reconciled: LocalInstance[] = [];

  for (const local of existing) {
    let next: LocalInstance = local;

    if (!next.serviceId && next.requestId != null) {
      const resolvedServiceId = serviceIdsByRequestId.get(next.requestId);
      if (resolvedServiceId) {
        next = {
          ...next,
          serviceId: resolvedServiceId,
        };
      }
    }

    const api = next.sandboxId ? apiById.get(next.sandboxId) : undefined;
    if (api) {
      matchedApiIds.add(api.id);
      reconciled.push({
        ...next,
        sandboxId: api.id,
        operator: api.managing_operator || next.operator,
        sidecarUrl: api.sidecar_url || next.sidecarUrl,
        image: api.image || next.image,
        agentIdentifier: api.agent_identifier || next.agentIdentifier,
        teeEnabled: next.teeEnabled || !!api.tee_deployment_id,
        credentialsAvailable: api.credentials_available ?? next.credentialsAvailable,
        sshPort: api.ssh_port || next.sshPort,
        serviceId: api.service_id != null ? String(api.service_id) : next.serviceId,
        status: statusFromApi(api.state),
        errorMessage: undefined,
      });
      continue;
    }

    if (!next.sandboxId) {
      const inferredApi = getUniqueDraftMatch(next, apiResults, matchedApiIds);
      if (inferredApi) {
        matchedApiIds.add(inferredApi.id);
        reconciled.push({
          ...next,
          sandboxId: inferredApi.id,
          operator: inferredApi.managing_operator || next.operator,
          sidecarUrl: inferredApi.sidecar_url || next.sidecarUrl,
          image: inferredApi.image || next.image,
          agentIdentifier: inferredApi.agent_identifier || next.agentIdentifier,
          teeEnabled: next.teeEnabled || !!inferredApi.tee_deployment_id,
          credentialsAvailable: inferredApi.credentials_available ?? next.credentialsAvailable,
          sshPort: inferredApi.ssh_port || next.sshPort,
          serviceId: inferredApi.service_id != null ? String(inferredApi.service_id) : next.serviceId,
          status: statusFromApi(inferredApi.state),
          errorMessage: undefined,
        });
        continue;
      }
    }

    reconciled.push(next);
  }

  for (const api of apiResults) {
    if (matchedApiIds.has(api.id)) continue;
    reconciled.push(instanceFromApi(api));
  }

  const seenIds = new Set<string>();
  const seenSandboxIds = new Set<string>();
  return reconciled.filter((instance) => {
    if (seenIds.has(instance.id)) return false;
    if (instance.sandboxId && seenSandboxIds.has(instance.sandboxId)) return false;
    seenIds.add(instance.id);
    if (instance.sandboxId) seenSandboxIds.add(instance.sandboxId);
    return true;
  });
}
