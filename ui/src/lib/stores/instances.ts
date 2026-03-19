import { computed } from 'nanostores';
import { persistedAtom } from '@tangle-network/blueprint-ui';

export interface LocalInstance {
  id: string;
  sandboxId?: string;
  requestId?: number;
  name: string;
  image: string;
  cpuCores: number;
  memoryMb: number;
  diskGb: number;
  createdAt: number;
  blueprintId: string;
  serviceId: string;
  operator?: string;
  sidecarUrl?: string;
  teeEnabled?: boolean;
  /** Agent identifier configured for the sidecar image. Must match a registered agent inside that image. */
  agentIdentifier?: string;
  status: 'creating' | 'running' | 'stopped' | 'gone' | 'error';
  txHash?: string;
  callId?: number;
  errorMessage?: string;
}

export const instanceListStore = persistedAtom<LocalInstance[]>({
  key: 'sandbox_cloud_instances',
  initial: [],
});

export const runningInstances = computed(instanceListStore, (list) =>
  list.filter((s) => s.status === 'running'),
);

export const activeInstances = computed(instanceListStore, (list) =>
  list.filter((s) => s.status !== 'gone'),
);

export function matchesInstanceKey(instance: LocalInstance, key: string): boolean {
  return instance.id === key || instance.sandboxId === key;
}

function dedupeInstances(records: LocalInstance[]): LocalInstance[] {
  const seenIds = new Set<string>();
  const seenSandboxIds = new Set<string>();
  const deduped: LocalInstance[] = [];

  for (const record of records) {
    if (seenIds.has(record.id)) continue;
    if (record.sandboxId && seenSandboxIds.has(record.sandboxId)) continue;
    seenIds.add(record.id);
    if (record.sandboxId) seenSandboxIds.add(record.sandboxId);
    deduped.push(record);
  }

  return deduped;
}

function setInstances(records: LocalInstance[]) {
  instanceListStore.set(dedupeInstances(records));
}

export function addInstance(instance: LocalInstance) {
  const existing = instanceListStore.get();
  if (existing.some((s) => s.id === instance.id || (!!instance.sandboxId && s.sandboxId === instance.sandboxId))) {
    return;
  }
  setInstances([instance, ...existing]);
}

export function updateInstance(id: string, extra: Partial<LocalInstance>) {
  setInstances(
    instanceListStore.get().map((instance) =>
      matchesInstanceKey(instance, id) ? { ...instance, ...extra } : instance,
    ),
  );
}

export function updateInstanceStatus(id: string, status: LocalInstance['status'], extra?: Partial<LocalInstance>) {
  setInstances(
    instanceListStore.get().map((instance) =>
      matchesInstanceKey(instance, id) ? { ...instance, ...extra, status } : instance,
    ),
  );
}

export function getInstance(id: string): LocalInstance | undefined {
  return instanceListStore.get().find((instance) => matchesInstanceKey(instance, id));
}

export function removeInstance(id: string) {
  setInstances(instanceListStore.get().filter((instance) => !matchesInstanceKey(instance, id)));
}
