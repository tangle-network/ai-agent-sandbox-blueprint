import { describe, expect, it } from 'vitest';

import { normalizeWorkflowConfig } from './useSandboxReads';

describe('normalizeWorkflowConfig', () => {
  it('converts bigint workflow fields into serializable numbers', () => {
    const normalized = normalizeWorkflowConfig({
      name: 'workflow-qa',
      workflow_json: '{}',
      trigger_type: 'cron',
      trigger_config: '*/15 * * * * *',
      sandbox_config_json: '{}',
      active: true,
      created_at: 123n,
      updated_at: 456n,
      last_triggered_at: 789n,
    });

    expect(normalized.created_at).toBe(123);
    expect(normalized.updated_at).toBe(456);
    expect(normalized.last_triggered_at).toBe(789);
    expect(() => JSON.stringify(normalized)).not.toThrow();
  });
});
