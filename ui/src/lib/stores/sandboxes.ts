import { atom, computed } from 'nanostores';
import { persistedAtom } from './persistedAtom';

/**
 * Local sandbox registry — tracks sandboxes the user has created or interacted with.
 * Since the contract only has per-sandbox lookups (no list), we maintain this locally
 * and hydrate on-chain status via contract reads.
 */

export interface LocalSandbox {
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
  /** Local status (hydrated from contract + events) */
  status: 'creating' | 'running' | 'stopped' | 'warm' | 'cold' | 'gone' | 'error';
  txHash?: string;
}

export const sandboxListStore = persistedAtom<LocalSandbox[]>({
  key: 'sandbox_cloud_sandboxes',
  initial: [],
});

export const runningSandboxes = computed(sandboxListStore, (list) =>
  list.filter((s) => s.status === 'running'),
);

export const stoppedSandboxes = computed(sandboxListStore, (list) =>
  list.filter((s) => s.status === 'stopped' || s.status === 'warm'),
);

export const activeSandboxes = computed(sandboxListStore, (list) =>
  list.filter((s) => s.status !== 'gone'),
);

export function addSandbox(sandbox: LocalSandbox) {
  const existing = sandboxListStore.get();
  if (existing.some((s) => s.id === sandbox.id)) return;
  sandboxListStore.set([sandbox, ...existing]);
}

export function updateSandboxStatus(id: string, status: LocalSandbox['status'], extra?: Partial<LocalSandbox>) {
  sandboxListStore.set(
    sandboxListStore.get().map((s) =>
      s.id === id ? { ...s, ...extra, status } : s,
    ),
  );
}

export function removeSandbox(id: string) {
  sandboxListStore.set(sandboxListStore.get().filter((s) => s.id !== id));
}

export function getSandbox(id: string): LocalSandbox | undefined {
  return sandboxListStore.get().find((s) => s.id === id);
}
