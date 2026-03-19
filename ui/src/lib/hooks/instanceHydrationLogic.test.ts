import { describe, expect, it } from 'vitest';
import { reconcileInstances } from './instanceHydrationLogic';
import type { LocalInstance } from '~/lib/stores/instances';
import type { ApiSandbox } from './sandboxHydrationLogic';

function makeInstance(overrides: Partial<LocalInstance> = {}): LocalInstance {
  return {
    id: 'draft-instance',
    name: 'draft-instance',
    image: 'agent-dev',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-instance-blueprint',
    serviceId: '',
    status: 'creating',
    ...overrides,
  };
}

function makeApiSandbox(overrides: Partial<ApiSandbox> = {}): ApiSandbox {
  return {
    id: 'sandbox-live-1',
    name: 'draft-instance',
    sidecar_url: 'http://127.0.0.1:9202',
    state: 'running',
    image: 'agent-dev',
    cpu_cores: 2,
    memory_mb: 2048,
    disk_gb: 10,
    created_at: Math.floor(Date.now() / 1000),
    last_activity_at: Math.floor(Date.now() / 1000),
    service_id: 3,
    ...overrides,
  };
}

describe('reconcileInstances', () => {
  it('resolves requestId to serviceId before operator hydration lands', () => {
    const reconciled = reconcileInstances(
      [makeInstance({ requestId: 7 })],
      [],
      new Map([[7, '3']]),
    );

    expect(reconciled[0].serviceId).toBe('3');
    expect(reconciled[0].requestId).toBe(7);
  });

  it('attaches canonical sandbox data to a matching draft', () => {
    const local = makeInstance({
      id: 'my-instance',
      createdAt: 1_700_000_000_000,
      requestId: 7,
      serviceId: '3',
    });
    const api = makeApiSandbox({
      created_at: 1_700_000_000,
      service_id: 3,
    });

    const reconciled = reconcileInstances([local], [api], new Map());

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].id).toBe('my-instance');
    expect(reconciled[0].sandboxId).toBe('sandbox-live-1');
    expect(reconciled[0].serviceId).toBe('3');
    expect(reconciled[0].status).toBe('running');
  });

  it('adds canonical instances that were not in local storage yet', () => {
    const reconciled = reconcileInstances([], [makeApiSandbox()], new Map());

    expect(reconciled).toHaveLength(1);
    expect(reconciled[0].id).toBe('sandbox-live-1');
    expect(reconciled[0].sandboxId).toBe('sandbox-live-1');
    expect(reconciled[0].serviceId).toBe('3');
  });
});
