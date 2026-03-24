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
  /** Whether the instance has AI credentials configured (e.g. ANTHROPIC_API_KEY). */
  credentialsAvailable?: boolean;
  status: 'creating' | 'running' | 'stopped' | 'gone' | 'error';
  txHash?: string;
  callId?: number;
  errorMessage?: string;
}

const INSTANCE_STORE_KEY_PREFIX = 'sandbox_cloud_instances';

type InstanceFingerprintEnv = Record<string, string | undefined>;

function normalizeFingerprintPart(value: string | undefined): string {
  return (value || '').trim().toLowerCase();
}

export function buildInstanceDeploymentFingerprint(env: InstanceFingerprintEnv = import.meta.env): string {
  const explicit = normalizeFingerprintPart(env.VITE_DEPLOYMENT_FINGERPRINT);
  if (explicit) return explicit;

  const fallback = [
    env.VITE_CHAIN_ID,
    env.VITE_TANGLE_CONTRACT,
    env.VITE_SANDBOX_BSM,
    env.VITE_INSTANCE_BSM,
    env.VITE_OPERATOR_API_URL,
    env.VITE_INSTANCE_OPERATOR_API_URL,
  ]
    .map(normalizeFingerprintPart)
    .filter(Boolean)
    .join('::');

  return fallback || 'default';
}

export function getInstanceStoreKey(fingerprint = buildInstanceDeploymentFingerprint()): string {
  return `${INSTANCE_STORE_KEY_PREFIX}::${fingerprint}`;
}

export function migrateLegacyInstanceCacheKey(
  storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>,
  currentKey: string,
) {
  if (currentKey === INSTANCE_STORE_KEY_PREFIX) return;

  const currentValue = storage.getItem(currentKey);
  const legacyValue = storage.getItem(INSTANCE_STORE_KEY_PREFIX);
  if (legacyValue == null) return;

  if (currentValue == null) {
    storage.setItem(currentKey, legacyValue);
  }

  storage.removeItem(INSTANCE_STORE_KEY_PREFIX);
}

export function pruneInstanceCacheKeys(storage: Pick<Storage, 'length' | 'key' | 'removeItem'>, currentKey: string) {
  const keys: string[] = [];
  for (let i = 0; i < storage.length; i += 1) {
    const key = storage.key(i);
    if (!key) continue;
    if (key === INSTANCE_STORE_KEY_PREFIX || key.startsWith(`${INSTANCE_STORE_KEY_PREFIX}::`)) {
      keys.push(key);
    }
  }

  keys
    .filter((key) => key !== currentKey)
    .forEach((key) => storage.removeItem(key));
}

const instanceDeploymentFingerprint = buildInstanceDeploymentFingerprint();
const instanceStoreKey = getInstanceStoreKey(instanceDeploymentFingerprint);

if (typeof window !== 'undefined' && window.localStorage) {
  migrateLegacyInstanceCacheKey(window.localStorage, instanceStoreKey);
  pruneInstanceCacheKeys(window.localStorage, instanceStoreKey);
}

export const instanceListStore = persistedAtom<LocalInstance[]>({
  key: instanceStoreKey,
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
