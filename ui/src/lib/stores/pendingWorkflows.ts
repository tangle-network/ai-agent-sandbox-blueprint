import { persistedAtom } from '@tangle-network/blueprint-ui';

import { buildSandboxDeploymentFingerprint } from './sandboxes';
import type { WorkflowBlueprintId, WorkflowScope } from '~/lib/workflows';

export type PendingWorkflowStatus =
  | 'processing'
  | 'awaiting-auth'
  | 'timed-out';

export interface PendingWorkflowCreation {
  key: string;
  ownerAddress: string;
  workflowId: number;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  operatorUrl: string;
  name: string;
  triggerType: string;
  triggerConfig: string;
  targetKind: number;
  targetSandboxId: string;
  targetServiceId: number;
  targetLabel: string;
  kindLabel: string;
  txHash: `0x${string}`;
  createdAt: number;
  submittedAt: number;
  status: PendingWorkflowStatus;
  statusMessage?: string;
}

const PENDING_WORKFLOW_STORE_KEY_PREFIX = 'sandbox_pending_workflows';

export function normalizeWorkflowOwnerAddress(address: string | undefined | null): string {
  return (address || '').trim().toLowerCase();
}

export function buildPendingWorkflowKey(
  ownerAddress: string,
  scope: WorkflowScope,
  workflowId: number,
): string {
  return `${normalizeWorkflowOwnerAddress(ownerAddress)}::${scope}::${workflowId}`;
}

function getPendingWorkflowStoreKey() {
  return `${PENDING_WORKFLOW_STORE_KEY_PREFIX}::${buildSandboxDeploymentFingerprint()}`;
}

function prunePendingWorkflowCacheKeys(
  storage: Pick<Storage, 'length' | 'key' | 'removeItem'>,
  currentKey: string,
) {
  const keys: string[] = [];
  for (let i = 0; i < storage.length; i += 1) {
    const key = storage.key(i);
    if (!key) continue;
    if (key === PENDING_WORKFLOW_STORE_KEY_PREFIX || key.startsWith(`${PENDING_WORKFLOW_STORE_KEY_PREFIX}::`)) {
      keys.push(key);
    }
  }

  keys
    .filter((key) => key !== currentKey)
    .forEach((key) => storage.removeItem(key));
}

const pendingWorkflowStoreKey = getPendingWorkflowStoreKey();

if (typeof window !== 'undefined' && window.localStorage) {
  prunePendingWorkflowCacheKeys(window.localStorage, pendingWorkflowStoreKey);
}

export const pendingWorkflowStore = persistedAtom<PendingWorkflowCreation[]>({
  key: pendingWorkflowStoreKey,
  initial: [],
});

function setPendingWorkflows(entries: PendingWorkflowCreation[]) {
  const deduped = new Map<string, PendingWorkflowCreation>();
  for (const entry of entries) {
    deduped.set(entry.key, entry);
  }
  pendingWorkflowStore.set(Array.from(deduped.values()));
}

export function addPendingWorkflow(entry: PendingWorkflowCreation) {
  const existing = pendingWorkflowStore.get();
  if (existing.some((record) => record.key === entry.key)) {
    setPendingWorkflows(existing.map((record) => (record.key === entry.key ? entry : record)));
    return;
  }

  setPendingWorkflows([entry, ...existing]);
}

export function updatePendingWorkflow(
  key: string,
  patch: Partial<PendingWorkflowCreation>,
) {
  setPendingWorkflows(
    pendingWorkflowStore.get().map((entry) => (
      entry.key === key ? { ...entry, ...patch } : entry
    )),
  );
}

export function removePendingWorkflow(key: string) {
  setPendingWorkflows(pendingWorkflowStore.get().filter((entry) => entry.key !== key));
}

export function removePendingWorkflowsByOwner(ownerAddress: string | undefined | null) {
  const normalizedOwner = normalizeWorkflowOwnerAddress(ownerAddress);
  setPendingWorkflows(
    pendingWorkflowStore.get().filter((entry) => entry.ownerAddress !== normalizedOwner),
  );
}
