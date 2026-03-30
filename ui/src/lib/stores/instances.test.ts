import { describe, it, expect, beforeEach } from 'vitest';
import {
  instanceListStore,
  addInstance,
  updateInstance,
  updateInstanceStatus,
  removeInstance,
  getInstance,
  runningInstances,
  activeInstances,
  buildInstanceDeploymentFingerprint,
  getInstanceStoreKey,
  migrateLegacyInstanceCacheKey,
  pruneInstanceCacheKeys,
  type LocalInstance,
} from './instances';

function makeInstance(overrides: Partial<LocalInstance> = {}): LocalInstance {
  return {
    id: 'inst-1',
    name: 'test-instance',
    image: 'ubuntu:22.04',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-instance-blueprint',
    serviceId: '2',
    status: 'running',
    ...overrides,
  };
}

beforeEach(() => {
  window.localStorage.clear();
  instanceListStore.set([]);
});

describe('instance cache versioning', () => {
  it('prefers the explicit deployment fingerprint when provided', () => {
    expect(buildInstanceDeploymentFingerprint({
      VITE_DEPLOYMENT_FINGERPRINT: 'local-abc-123',
      VITE_CHAIN_ID: '31337',
    })).toBe('local-abc-123');
  });

  it('falls back to environment details when no explicit fingerprint is set', () => {
    expect(buildInstanceDeploymentFingerprint({
      VITE_CHAIN_ID: '31337',
      VITE_TANGLE_CONTRACT: '0xabc',
      VITE_SANDBOX_BSM: '0xdef',
      VITE_OPERATOR_API_URL: 'http://127.0.0.1:9102',
    })).toBe('31337::0xabc::0xdef::http://127.0.0.1:9102');
  });

  it('migrates the legacy global key into the active deployment key', () => {
    const currentKey = getInstanceStoreKey('deploy-new');
    window.localStorage.setItem('sandbox_cloud_instances', JSON.stringify([{ id: 'legacy-inst' }]));

    migrateLegacyInstanceCacheKey(window.localStorage, currentKey);

    expect(window.localStorage.getItem('sandbox_cloud_instances')).toBeNull();
    expect(window.localStorage.getItem(currentKey)).toBe(JSON.stringify([{ id: 'legacy-inst' }]));
  });

  it('prunes stale cache keys while keeping the active deployment key', () => {
    const currentKey = getInstanceStoreKey('deploy-new');
    window.localStorage.setItem(getInstanceStoreKey('deploy-old'), JSON.stringify([{ id: 'old' }]));
    window.localStorage.setItem(currentKey, JSON.stringify([{ id: 'current' }]));

    pruneInstanceCacheKeys(window.localStorage, currentKey);

    expect(window.localStorage.getItem(getInstanceStoreKey('deploy-old'))).toBeNull();
    expect(window.localStorage.getItem(currentKey)).toBe(JSON.stringify([{ id: 'current' }]));
  });
});

describe('addInstance', () => {
  it('adds a new instance', () => {
    addInstance(makeInstance({ id: 'inst-1' }));
    expect(instanceListStore.get()).toHaveLength(1);
  });

  it('deduplicates by id', () => {
    addInstance(makeInstance({ id: 'inst-1', name: 'first' }));
    addInstance(makeInstance({ id: 'inst-1', name: 'second' }));
    expect(instanceListStore.get()).toHaveLength(1);
    expect(instanceListStore.get()[0].name).toBe('first');
  });

  it('prepends new instances', () => {
    addInstance(makeInstance({ id: 'inst-1' }));
    addInstance(makeInstance({ id: 'inst-2' }));
    expect(instanceListStore.get()[0].id).toBe('inst-2');
  });
});

describe('updateInstanceStatus', () => {
  it('updates status and merges extra fields', () => {
    addInstance(makeInstance({ id: 'inst-1', status: 'creating' }));
    updateInstanceStatus('inst-1', 'running', { sidecarUrl: 'http://sidecar:9090' });
    const inst = getInstance('inst-1')!;
    expect(inst.status).toBe('running');
    expect(inst.sidecarUrl).toBe('http://sidecar:9090');
  });

  it('no-op for unknown id', () => {
    addInstance(makeInstance({ id: 'inst-1' }));
    updateInstanceStatus('unknown', 'gone');
    expect(getInstance('inst-1')?.status).toBe('running');
  });

  it('updates by sandboxId when the route key stays stable', () => {
    addInstance(makeInstance({ id: 'draft-name', sandboxId: 'sandbox-live-1', status: 'creating' }));
    updateInstanceStatus('sandbox-live-1', 'running', { sidecarUrl: 'http://sidecar:9090' });
    const inst = getInstance('draft-name')!;
    expect(inst.status).toBe('running');
    expect(inst.sidecarUrl).toBe('http://sidecar:9090');
  });
});

describe('updateInstance', () => {
  it('merges metadata without changing status', () => {
    addInstance(makeInstance({ id: 'inst-1', status: 'creating' }));
    updateInstance('inst-1', { requestId: 7, serviceId: '3' });
    const inst = getInstance('inst-1')!;
    expect(inst.status).toBe('creating');
    expect(inst.requestId).toBe(7);
    expect(inst.serviceId).toBe('3');
  });
});

describe('removeInstance', () => {
  it('removes by id', () => {
    addInstance(makeInstance({ id: 'inst-1' }));
    addInstance(makeInstance({ id: 'inst-2' }));
    removeInstance('inst-1');
    expect(instanceListStore.get()).toHaveLength(1);
    expect(getInstance('inst-1')).toBeUndefined();
  });

  it('removes by sandboxId', () => {
    addInstance(makeInstance({ id: 'draft-name', sandboxId: 'sandbox-live-1' }));
    removeInstance('sandbox-live-1');
    expect(instanceListStore.get()).toHaveLength(0);
  });
});

describe('runningInstances', () => {
  it('filters only running', () => {
    addInstance(makeInstance({ id: 'inst-1', status: 'running' }));
    addInstance(makeInstance({ id: 'inst-2', status: 'stopped' }));
    expect(runningInstances.get()).toHaveLength(1);
    expect(runningInstances.get()[0].id).toBe('inst-1');
  });
});

describe('activeInstances', () => {
  it('excludes only gone', () => {
    addInstance(makeInstance({ id: 'inst-1', status: 'running' }));
    addInstance(makeInstance({ id: 'inst-2', status: 'gone' }));
    addInstance(makeInstance({ id: 'inst-3', status: 'error' }));
    expect(activeInstances.get()).toHaveLength(2);
  });
});
