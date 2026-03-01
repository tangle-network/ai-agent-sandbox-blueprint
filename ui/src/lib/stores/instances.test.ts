import { describe, it, expect, beforeEach } from 'vitest';
import {
  instanceListStore,
  addInstance,
  updateInstanceStatus,
  removeInstance,
  getInstance,
  runningInstances,
  activeInstances,
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
  instanceListStore.set([]);
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
});

describe('removeInstance', () => {
  it('removes by id', () => {
    addInstance(makeInstance({ id: 'inst-1' }));
    addInstance(makeInstance({ id: 'inst-2' }));
    removeInstance('inst-1');
    expect(instanceListStore.get()).toHaveLength(1);
    expect(getInstance('inst-1')).toBeUndefined();
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
