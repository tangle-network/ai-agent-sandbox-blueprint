/**
 * Tests for teeEnabled field integration across stores and deploy flow.
 *
 * Verifies that teeEnabled is correctly stored, retrieved, and persisted
 * in both sandbox and instance stores.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import {
  sandboxListStore,
  addSandbox,
  getSandbox,
  updateSandboxStatus,
  removeSandbox,
  type LocalSandbox,
} from './sandboxes';
import {
  instanceListStore,
  addInstance,
  getInstance,
  updateInstanceStatus,
  type LocalInstance,
} from './instances';

function makeSandbox(overrides: Partial<LocalSandbox> & { id?: string } = {}): LocalSandbox {
  const { id, ...rest } = overrides;
  return {
    localId: id ?? `sb-${Math.random().toString(36).slice(2, 8)}`,
    name: 'test-sandbox',
    image: 'ubuntu:22.04',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-sandbox-blueprint',
    serviceId: '1',
    status: 'running',
    ...rest,
  };
}

function makeInstance(overrides: Partial<LocalInstance> = {}): LocalInstance {
  return {
    id: `inst-${Math.random().toString(36).slice(2, 8)}`,
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

describe('sandbox store: teeEnabled field', () => {
  beforeEach(() => {
    sandboxListStore.set([]);
  });

  it('stores teeEnabled=true on a sandbox', () => {
    const sb = makeSandbox({ id: 'sb-tee', teeEnabled: true });
    addSandbox(sb);
    expect(getSandbox('sb-tee')?.teeEnabled).toBe(true);
  });

  it('stores teeEnabled=undefined (non-TEE) by default', () => {
    const sb = makeSandbox({ id: 'sb-plain' });
    addSandbox(sb);
    expect(getSandbox('sb-plain')?.teeEnabled).toBeUndefined();
  });

  it('preserves teeEnabled through status updates', () => {
    const sb = makeSandbox({ id: 'sb-tee-update', teeEnabled: true, status: 'creating' });
    addSandbox(sb);

    updateSandboxStatus('sb-tee-update', 'running', { sidecarUrl: 'http://sidecar:8080' });

    const updated = getSandbox('sb-tee-update');
    expect(updated?.teeEnabled).toBe(true);
    expect(updated?.status).toBe('running');
    expect(updated?.sidecarUrl).toBe('http://sidecar:8080');
  });

  it('preserves teeEnabled=false explicitly', () => {
    const sb = makeSandbox({ id: 'sb-explicit-false', teeEnabled: false });
    addSandbox(sb);
    expect(getSandbox('sb-explicit-false')?.teeEnabled).toBe(false);
  });

  it('can coexist with TEE and non-TEE sandboxes', () => {
    addSandbox(makeSandbox({ id: 'sb-tee-1', teeEnabled: true }));
    addSandbox(makeSandbox({ id: 'sb-plain-1' }));
    addSandbox(makeSandbox({ id: 'sb-tee-2', teeEnabled: true }));

    const all = sandboxListStore.get();
    const tee = all.filter((s) => s.teeEnabled);
    const plain = all.filter((s) => !s.teeEnabled);

    expect(tee).toHaveLength(2);
    expect(plain).toHaveLength(1);
  });
});

describe('instance store: teeEnabled field', () => {
  beforeEach(() => {
    instanceListStore.set([]);
  });

  it('stores teeEnabled=true on an instance', () => {
    const inst = makeInstance({
      id: 'inst-tee',
      teeEnabled: true,
      blueprintId: 'ai-agent-tee-instance-blueprint',
    });
    addInstance(inst);
    expect(getInstance('inst-tee')?.teeEnabled).toBe(true);
  });

  it('stores teeEnabled=false for non-TEE instance', () => {
    const inst = makeInstance({ id: 'inst-plain', teeEnabled: false });
    addInstance(inst);
    expect(getInstance('inst-plain')?.teeEnabled).toBe(false);
  });

  it('preserves teeEnabled through status updates', () => {
    const inst = makeInstance({ id: 'inst-tee-update', teeEnabled: true, status: 'creating' });
    addInstance(inst);

    updateInstanceStatus('inst-tee-update', 'running', { sidecarUrl: 'http://sidecar:9090' });

    const updated = getInstance('inst-tee-update');
    expect(updated?.teeEnabled).toBe(true);
    expect(updated?.status).toBe('running');
  });
});
