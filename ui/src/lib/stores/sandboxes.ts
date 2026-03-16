import { atom, computed } from 'nanostores';
import { persistedAtom } from '@tangle-network/blueprint-ui';

/**
 * Local sandbox registry — tracks sandboxes the user has created or interacted with.
 * Since the contract only has per-sandbox lookups (no list), we maintain this locally
 * and hydrate on-chain status via contract reads.
 */

export interface LocalSandbox {
  localId: string;
  sandboxId?: string;
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
  /** Agent identifier — when non-empty, the sandbox has an AI agent and chat is available. */
  agentIdentifier?: string;
  /** Local status (hydrated from contract + events) */
  status: 'creating' | 'running' | 'stopped' | 'warm' | 'cold' | 'gone' | 'error';
  txHash?: string;
  callId?: number;
}

interface LegacySandboxRecord extends Partial<LocalSandbox> {
  id?: string;
}

const DRAFT_PREFIX = 'draft:';
const LEGACY_PREFIX = 'legacy:';
const CANONICAL_PREFIX = 'canonical:';

export function isCanonicalSandboxId(id: string | undefined): id is string {
  return !!id && !id.startsWith(DRAFT_PREFIX) && !id.startsWith(LEGACY_PREFIX);
}

function shouldPromoteLegacyId(record: LegacySandboxRecord): boolean {
  if (!record.id) return false;
  if (record.status === 'creating' || record.status === 'error') return false;
  return true;
}

export function createSandboxDraftId(name: string): string {
  const normalized = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 24) || 'sandbox';
  const nonce = Math.random().toString(36).slice(2, 8);
  return `${DRAFT_PREFIX}${normalized}:${Date.now().toString(36)}:${nonce}`;
}

export function getSandboxRouteKey(sandbox: Pick<LocalSandbox, 'localId' | 'sandboxId'>): string {
  return sandbox.sandboxId ?? sandbox.localId;
}

export function matchesSandboxKey(sandbox: LocalSandbox, key: string): boolean {
  return sandbox.localId === key || sandbox.sandboxId === key;
}

export function normalizeSandbox(record: LegacySandboxRecord): LocalSandbox {
  const canonicalId = record.sandboxId
    || (shouldPromoteLegacyId(record) ? record.id : undefined);
  const localId = record.localId
    || (record.id
      ? `${canonicalId ? CANONICAL_PREFIX : LEGACY_PREFIX}${record.id}`
      : createSandboxDraftId(record.name || 'sandbox'));

  return {
    localId,
    sandboxId: canonicalId,
    name: record.name || canonicalId || record.id || 'sandbox',
    image: record.image || '',
    cpuCores: record.cpuCores ?? 2,
    memoryMb: record.memoryMb ?? 2048,
    diskGb: record.diskGb ?? 10,
    createdAt: record.createdAt ?? Date.now(),
    blueprintId: record.blueprintId || '',
    serviceId: record.serviceId || '',
    operator: record.operator,
    sidecarUrl: record.sidecarUrl,
    teeEnabled: record.teeEnabled,
    agentIdentifier: record.agentIdentifier,
    status: record.status ?? 'creating',
    txHash: record.txHash,
    callId: record.callId,
  };
}

export function normalizeSandboxList(records: LegacySandboxRecord[]): LocalSandbox[] {
  return records.map(normalizeSandbox);
}

function dedupeSandboxes(records: LocalSandbox[]): LocalSandbox[] {
  const seenLocalIds = new Set<string>();
  const seenSandboxIds = new Set<string>();
  const deduped: LocalSandbox[] = [];

  for (const record of records) {
    if (seenLocalIds.has(record.localId)) continue;
    if (record.sandboxId && seenSandboxIds.has(record.sandboxId)) continue;
    seenLocalIds.add(record.localId);
    if (record.sandboxId) seenSandboxIds.add(record.sandboxId);
    deduped.push(record);
  }

  return deduped;
}

function setSandboxes(records: LocalSandbox[]) {
  sandboxListStore.set(dedupeSandboxes(records));
}

export const sandboxListStore = persistedAtom<LocalSandbox[]>({
  key: 'sandbox_cloud_sandboxes',
  initial: [],
});

const normalizedExisting = normalizeSandboxList(sandboxListStore.get() as LegacySandboxRecord[]);
const existingSerialized = JSON.stringify(sandboxListStore.get());
const normalizedSerialized = JSON.stringify(normalizedExisting);
if (normalizedSerialized !== existingSerialized) {
  sandboxListStore.set(normalizedExisting);
}

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
  const record = normalizeSandbox(sandbox);
  const existing = sandboxListStore.get();
  if (existing.some((s) => s.localId === record.localId || (record.sandboxId && s.sandboxId === record.sandboxId))) {
    return;
  }
  setSandboxes([record, ...existing]);
}

export function updateSandboxStatus(key: string, status: LocalSandbox['status'], extra?: Partial<LocalSandbox>) {
  const next = sandboxListStore.get().map((sandbox) => {
    if (!matchesSandboxKey(sandbox, key)) return sandbox;
    return normalizeSandbox({
      ...sandbox,
      ...extra,
      status,
    });
  });
  setSandboxes(next);
}

export function removeSandbox(key: string) {
  setSandboxes(sandboxListStore.get().filter((sandbox) => !matchesSandboxKey(sandbox, key)));
}

export function findSandboxByKey(sandboxes: LocalSandbox[], key: string): LocalSandbox | undefined {
  return sandboxes.find((sandbox) => matchesSandboxKey(sandbox, key));
}

export function getSandbox(key: string): LocalSandbox | undefined {
  return findSandboxByKey(sandboxListStore.get(), key);
}
