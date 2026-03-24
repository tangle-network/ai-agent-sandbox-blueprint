import { describe, expect, it } from 'vitest';

import { filterOwnedWorkflowIds, normalizeWorkflowConfig } from './useSandboxReads';

describe('normalizeWorkflowConfig', () => {
  it('converts bigint workflow fields into serializable numbers', () => {
    const normalized = normalizeWorkflowConfig({
      name: 'workflow-qa',
      workflow_json: '{}',
      trigger_type: 'cron',
      trigger_config: '*/15 * * * * *',
      sandbox_config_json: '{}',
      target_kind: 0,
      target_sandbox_id: 'sb-1',
      target_service_id: 42n,
      active: true,
      created_at: 123n,
      updated_at: 456n,
      last_triggered_at: 789n,
    });

    expect(normalized.target_service_id).toBe('42');
    expect(normalized.created_at).toBe(123);
    expect(normalized.updated_at).toBe(456);
    expect(normalized.last_triggered_at).toBe(789);
    expect(() => JSON.stringify(normalized)).not.toThrow();
  });
});

describe('filterOwnedWorkflowIds', () => {
  it('keeps only workflows created by the connected owner', () => {
    const owned = filterOwnedWorkflowIds(
      [1n, 2n, 3n],
      [
        {
          callId: 1n,
          caller: '0x1111111111111111111111111111111111111111',
        },
        {
          callId: 3n,
          caller: '0x2222222222222222222222222222222222222222',
        },
        {
          callId: 99n,
          caller: '0x1111111111111111111111111111111111111111',
        },
      ],
      '0x1111111111111111111111111111111111111111',
    );

    expect(owned).toEqual([1n]);
  });

  it('matches owners case-insensitively', () => {
    const owned = filterOwnedWorkflowIds(
      [7n],
      [
        {
          callId: 7n,
          caller: '0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa',
        },
      ],
      '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    );

    expect(owned).toEqual([7n]);
  });
});
