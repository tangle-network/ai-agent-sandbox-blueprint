import { describe, it, expect, beforeEach } from 'vitest';
import {
  sandboxListStore,
  addSandbox,
  updateSandboxStatus,
  removeSandbox,
  getSandbox,
  runningSandboxes,
  stoppedSandboxes,
  activeSandboxes,
  buildSandboxDeploymentFingerprint,
  getSandboxStoreKey,
  pruneSandboxCacheKeys,
  type LocalSandbox,
} from './sandboxes';

function makeSandbox(overrides: Partial<LocalSandbox> & { id?: string } = {}): LocalSandbox {
  const { id, ...rest } = overrides;
  return {
    localId: id ?? 'sb-1',
    name: 'test',
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

beforeEach(() => {
  window.localStorage.clear();
  sandboxListStore.set([]);
});

describe('sandbox cache versioning', () => {
  it('prefers the explicit deployment fingerprint when provided', () => {
    expect(buildSandboxDeploymentFingerprint({
      VITE_DEPLOYMENT_FINGERPRINT: 'local-abc-123',
      VITE_CHAIN_ID: '31337',
    })).toBe('local-abc-123');
  });

  it('falls back to environment details when no explicit fingerprint is set', () => {
    expect(buildSandboxDeploymentFingerprint({
      VITE_CHAIN_ID: '31337',
      VITE_TANGLE_CONTRACT: '0xabc',
      VITE_SANDBOX_BSM: '0xdef',
      VITE_OPERATOR_API_URL: 'http://127.0.0.1:9102',
    })).toBe('31337::0xabc::0xdef::http://127.0.0.1:9102');
  });

  it('prunes legacy and stale cache keys while keeping the active deployment key', () => {
    const currentKey = getSandboxStoreKey('deploy-new');
    window.localStorage.setItem('sandbox_cloud_sandboxes', JSON.stringify([{ localId: 'legacy' }]));
    window.localStorage.setItem(getSandboxStoreKey('deploy-old'), JSON.stringify([{ localId: 'old' }]));
    window.localStorage.setItem(currentKey, JSON.stringify([{ localId: 'current' }]));

    pruneSandboxCacheKeys(window.localStorage, currentKey);

    expect(window.localStorage.getItem('sandbox_cloud_sandboxes')).toBeNull();
    expect(window.localStorage.getItem(getSandboxStoreKey('deploy-old'))).toBeNull();
    expect(window.localStorage.getItem(currentKey)).toBe(JSON.stringify([{ localId: 'current' }]));
  });
});

// ── addSandbox ──

describe('addSandbox', () => {
  it('adds a new sandbox to the list', () => {
    addSandbox(makeSandbox({ id: 'sb-1' }));
    expect(sandboxListStore.get()).toHaveLength(1);
    expect(sandboxListStore.get()[0].localId).toBe('sb-1');
  });

  it('prepends new sandbox (most recent first)', () => {
    addSandbox(makeSandbox({ id: 'sb-1' }));
    addSandbox(makeSandbox({ id: 'sb-2' }));
    expect(sandboxListStore.get()[0].localId).toBe('sb-2');
    expect(sandboxListStore.get()[1].localId).toBe('sb-1');
  });

  it('deduplicates by id — second add is no-op', () => {
    addSandbox(makeSandbox({ id: 'sb-1', name: 'first' }));
    addSandbox(makeSandbox({ id: 'sb-1', name: 'second' }));
    expect(sandboxListStore.get()).toHaveLength(1);
    expect(sandboxListStore.get()[0].name).toBe('first');
  });

  it('allows different IDs', () => {
    addSandbox(makeSandbox({ id: 'sb-1' }));
    addSandbox(makeSandbox({ id: 'sb-2' }));
    expect(sandboxListStore.get()).toHaveLength(2);
  });
});

// ── updateSandboxStatus ──

describe('updateSandboxStatus', () => {
  it('updates status of matching sandbox', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'running' }));
    updateSandboxStatus('sb-1', 'stopped');
    expect(getSandbox('sb-1')?.status).toBe('stopped');
  });

  it('merges extra fields', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'creating' }));
    updateSandboxStatus('sb-1', 'running', { sidecarUrl: 'http://test:9090' });
    const sb = getSandbox('sb-1')!;
    expect(sb.status).toBe('running');
    expect(sb.sidecarUrl).toBe('http://test:9090');
  });

  it('does not affect other sandboxes', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'running' }));
    addSandbox(makeSandbox({ id: 'sb-2', status: 'running' }));
    updateSandboxStatus('sb-1', 'stopped');
    expect(getSandbox('sb-2')?.status).toBe('running');
  });

  it('is no-op for unknown id', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'running' }));
    updateSandboxStatus('sb-unknown', 'gone');
    expect(sandboxListStore.get()).toHaveLength(1);
    expect(getSandbox('sb-1')?.status).toBe('running');
  });
});

// ── removeSandbox ──

describe('removeSandbox', () => {
  it('removes sandbox by id', () => {
    addSandbox(makeSandbox({ id: 'sb-1' }));
    addSandbox(makeSandbox({ id: 'sb-2' }));
    removeSandbox('sb-1');
    expect(sandboxListStore.get()).toHaveLength(1);
    expect(getSandbox('sb-1')).toBeUndefined();
    expect(getSandbox('sb-2')).toBeDefined();
  });

  it('is no-op for unknown id', () => {
    addSandbox(makeSandbox({ id: 'sb-1' }));
    removeSandbox('sb-unknown');
    expect(sandboxListStore.get()).toHaveLength(1);
  });
});

// ── getSandbox ──

describe('getSandbox', () => {
  it('returns sandbox by id', () => {
    addSandbox(makeSandbox({ id: 'sb-1', name: 'my-sandbox' }));
    expect(getSandbox('sb-1')?.name).toBe('my-sandbox');
  });

  it('returns undefined for missing id', () => {
    expect(getSandbox('sb-missing')).toBeUndefined();
  });
});

// ── Computed stores ──

describe('runningSandboxes', () => {
  it('filters only running sandboxes', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'running' }));
    addSandbox(makeSandbox({ id: 'sb-2', status: 'stopped' }));
    addSandbox(makeSandbox({ id: 'sb-3', status: 'running' }));
    expect(runningSandboxes.get()).toHaveLength(2);
    expect(runningSandboxes.get().map((s) => s.localId).sort()).toEqual(['sb-1', 'sb-3']);
  });

  it('returns empty when none running', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'stopped' }));
    expect(runningSandboxes.get()).toHaveLength(0);
  });
});

describe('stoppedSandboxes', () => {
  it('includes stopped and warm statuses', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'stopped' }));
    addSandbox(makeSandbox({ id: 'sb-2', status: 'warm' }));
    addSandbox(makeSandbox({ id: 'sb-3', status: 'running' }));
    expect(stoppedSandboxes.get()).toHaveLength(2);
  });

  it('excludes gone, cold, error, creating', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'gone' }));
    addSandbox(makeSandbox({ id: 'sb-2', status: 'cold' }));
    addSandbox(makeSandbox({ id: 'sb-3', status: 'error' }));
    addSandbox(makeSandbox({ id: 'sb-4', status: 'creating' }));
    expect(stoppedSandboxes.get()).toHaveLength(0);
  });
});

describe('activeSandboxes', () => {
  it('excludes only gone status', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'running' }));
    addSandbox(makeSandbox({ id: 'sb-2', status: 'stopped' }));
    addSandbox(makeSandbox({ id: 'sb-3', status: 'gone' }));
    addSandbox(makeSandbox({ id: 'sb-4', status: 'error' }));
    expect(activeSandboxes.get()).toHaveLength(3);
    expect(activeSandboxes.get().every((s) => s.status !== 'gone')).toBe(true);
  });

  it('returns empty when all gone', () => {
    addSandbox(makeSandbox({ id: 'sb-1', status: 'gone' }));
    expect(activeSandboxes.get()).toHaveLength(0);
  });
});
