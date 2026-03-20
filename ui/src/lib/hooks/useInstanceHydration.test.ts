import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { LocalInstance } from '~/lib/stores/instances';

const { getTransactionReceipt } = vi.hoisted(() => ({
  getTransactionReceipt: vi.fn(),
}));

vi.mock('@tangle-network/blueprint-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tangle-network/blueprint-ui')>();
  return {
    ...actual,
    getAddresses: vi.fn(),
    publicClient: {
      getTransactionReceipt,
      getLogs: vi.fn(),
    },
    tangleServicesAbi: [],
  };
});

import { recoverDraftFromReceipt } from './useInstanceHydration';

function makeInstance(overrides: Partial<LocalInstance> = {}): LocalInstance {
  return {
    id: 'draft-instance',
    name: 'draft-instance',
    image: 'agent-dev:latest',
    cpuCores: 2,
    memoryMb: 2048,
    diskGb: 10,
    createdAt: Date.now(),
    blueprintId: 'ai-agent-instance-blueprint',
    serviceId: '',
    status: 'creating',
    txHash: '0xabc123',
    ...overrides,
  };
}

describe('recoverDraftFromReceipt', () => {
  beforeEach(() => {
    getTransactionReceipt.mockReset();
  });

  it('recovers a previously errored draft when the receipt contains ServiceRequested', async () => {
    getTransactionReceipt.mockResolvedValue({
      status: 'success',
      logs: [
        {
          data: '0x',
          topics: [
            '0xbd1fdda393b679e6c4f873e233b34e2c4ea8283a3f76345dbc143b86ea047679',
            '0x0000000000000000000000000000000000000000000000000000000000000003',
            '0x0000000000000000000000000000000000000000000000000000000000000002',
            '0x0000000000000000000000009965507d1a55bcc2695c58ba16fb37d819b0a4dc',
          ],
        },
      ],
    });

    const recovered = await recoverDraftFromReceipt(
      makeInstance({
        status: 'error',
        errorMessage: 'Instance transaction confirmed without a ServiceRequested event.',
      }),
      new AbortController().signal,
    );

    expect(recovered.status).toBe('creating');
    expect(recovered.requestId).toBe(3);
    expect(recovered.errorMessage).toBeUndefined();
  });
});
