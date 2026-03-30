import { describe, expect, it, vi } from 'vitest';
import { discoverOperatorsWithClient } from '../../../../../blueprint-ui/src/hooks/useOperators';

const SERVICES = '0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9';
const BLUEPRINT_ID = 2n;
const OP1 = '0x70997970C51812dc3A010C7d01b50e0d17dc79C8';
const OP2 = '0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC';

const logs = [
  {
    args: {
      blueprintId: BLUEPRINT_ID,
      operator: OP1,
      ecdsaPublicKey: '0xlog-op1',
      rpcAddress: 'http://log-op1',
    },
  },
  {
    args: {
      blueprintId: BLUEPRINT_ID,
      operator: OP2,
      ecdsaPublicKey: '0xlog-op2',
      rpcAddress: 'http://log-op2',
    },
  },
];

describe('discoverOperatorsWithClient', () => {
  it('uses multicall results when available', async () => {
    const client = {
      readContract: vi.fn().mockResolvedValue(2n),
      getLogs: vi.fn().mockResolvedValue(logs),
      multicall: vi.fn()
        .mockResolvedValueOnce([
          { status: 'success', result: true },
          { status: 'success', result: true },
        ])
        .mockResolvedValueOnce([
          { status: 'success', result: ['0xpref-op1', 'http://pref-op1'] },
          { status: 'success', result: ['0xpref-op2', 'http://pref-op2'] },
        ]),
    };

    const result = await discoverOperatorsWithClient(client as any, SERVICES, BLUEPRINT_ID);

    expect(result.operatorCount).toBe(2n);
    expect(result.operators).toEqual([
      { address: OP1, ecdsaPublicKey: '0xpref-op1', rpcAddress: 'http://pref-op1' },
      { address: OP2, ecdsaPublicKey: '0xpref-op2', rpcAddress: 'http://pref-op2' },
    ]);
    expect(client.readContract).toHaveBeenCalledTimes(1);
  });

  it('falls back to direct reads when multicall cannot verify operators', async () => {
    const client = {
      readContract: vi.fn(async ({ functionName, args }: any) => {
        if (functionName === 'blueprintOperatorCount') return 2n;
        if (functionName === 'isOperatorRegistered') {
          return args[1] === OP1 || args[1] === OP2;
        }
        if (functionName === 'getOperatorPreferences') {
          return args[1] === OP1
            ? ['0xpref-op1', 'http://pref-op1']
            : ['0xpref-op2', 'http://pref-op2'];
        }
        throw new Error(`unexpected function ${functionName}`);
      }),
      getLogs: vi.fn().mockResolvedValue(logs),
      multicall: vi.fn()
        .mockResolvedValueOnce([
          { status: 'failure', error: new Error('multicall unavailable') },
          { status: 'failure', error: new Error('multicall unavailable') },
        ])
        .mockResolvedValueOnce([
          { status: 'failure', error: new Error('multicall unavailable') },
          { status: 'failure', error: new Error('multicall unavailable') },
        ]),
    };

    const result = await discoverOperatorsWithClient(client as any, SERVICES, BLUEPRINT_ID);

    expect(result.operatorCount).toBe(2n);
    expect(result.operators).toEqual([
      { address: OP1, ecdsaPublicKey: '0xpref-op1', rpcAddress: 'http://pref-op1' },
      { address: OP2, ecdsaPublicKey: '0xpref-op2', rpcAddress: 'http://pref-op2' },
    ]);
    expect(client.readContract).toHaveBeenCalledTimes(5);
  });

  it('keeps log-derived preferences when direct preference reads fail', async () => {
    const client = {
      readContract: vi.fn(async ({ functionName, args }: any) => {
        if (functionName === 'blueprintOperatorCount') return 2n;
        if (functionName === 'isOperatorRegistered') return true;
        if (functionName === 'getOperatorPreferences' && args[1] === OP1) {
          throw new Error('preferences unavailable');
        }
        if (functionName === 'getOperatorPreferences' && args[1] === OP2) {
          return ['0xpref-op2', 'http://pref-op2'];
        }
        throw new Error(`unexpected function ${functionName}`);
      }),
      getLogs: vi.fn().mockResolvedValue(logs),
      multicall: vi.fn().mockRejectedValue(new Error('multicall unavailable')),
    };

    const result = await discoverOperatorsWithClient(client as any, SERVICES, BLUEPRINT_ID);

    expect(result.operators).toEqual([
      { address: OP1, ecdsaPublicKey: '0xlog-op1', rpcAddress: 'http://log-op1' },
      { address: OP2, ecdsaPublicKey: '0xpref-op2', rpcAddress: 'http://pref-op2' },
    ]);
  });

  it('throws when multicall and direct verification both fail', async () => {
    const client = {
      readContract: vi.fn(async ({ functionName }: any) => {
        if (functionName === 'blueprintOperatorCount') return 2n;
        throw new Error('direct verification failed');
      }),
      getLogs: vi.fn().mockResolvedValue(logs),
      multicall: vi.fn().mockRejectedValue(new Error('multicall unavailable')),
    };

    await expect(
      discoverOperatorsWithClient(client as any, SERVICES, BLUEPRINT_ID),
    ).rejects.toThrow('direct verification failed');
  });

  it('returns an empty result immediately when on-chain operator count is zero', async () => {
    const client = {
      readContract: vi.fn().mockResolvedValue(0n),
      getLogs: vi.fn(),
      multicall: vi.fn(),
    };

    const result = await discoverOperatorsWithClient(client as any, SERVICES, BLUEPRINT_ID);

    expect(result).toEqual({ operators: [], operatorCount: 0n });
    expect(client.getLogs).not.toHaveBeenCalled();
    expect(client.multicall).not.toHaveBeenCalled();
  });
});
