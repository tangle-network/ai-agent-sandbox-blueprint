import { computed } from 'nanostores';
import { persistedAtom } from './persistedAtom';

export interface LocalInstance {
  id: string;
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
  status: 'creating' | 'running' | 'stopped' | 'gone' | 'error';
  txHash?: string;
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

export function addInstance(instance: LocalInstance) {
  const existing = instanceListStore.get();
  if (existing.some((s) => s.id === instance.id)) return;
  instanceListStore.set([instance, ...existing]);
}

export function updateInstanceStatus(id: string, status: LocalInstance['status'], extra?: Partial<LocalInstance>) {
  instanceListStore.set(
    instanceListStore.get().map((s) =>
      s.id === id ? { ...s, ...extra, status } : s,
    ),
  );
}

export function removeInstance(id: string) {
  instanceListStore.set(instanceListStore.get().filter((s) => s.id !== id));
}
