import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getBlockNumber, getBlock } = vi.hoisted(() => ({
  getBlockNumber: vi.fn(),
  getBlock: vi.fn(),
}));

vi.mock('@tangle-network/blueprint-ui', () => ({
  publicClient: {
    getBlockNumber,
    getBlock,
  },
}));

import { walletRpcMatchesAppRpc } from './walletRpcSync';

function installEthereumRequest(
  implementation: (request: { method: string; params?: unknown[] }) => Promise<unknown>,
) {
  Object.defineProperty(window, 'ethereum', {
    value: { request: vi.fn(implementation) },
    configurable: true,
    writable: true,
  });

  return (window as any).ethereum.request as ReturnType<typeof vi.fn>;
}

describe('walletRpcMatchesAppRpc', () => {
  beforeEach(() => {
    getBlockNumber.mockReset();
    getBlock.mockReset();
    delete (window as any).ethereum;
  });

  it('returns true when hashes match at the shared fixed block height', async () => {
    const request = installEthereumRequest(async ({ method, params }) => {
      if (method === 'eth_chainId') return '0x7a69';
      if (method === 'eth_blockNumber') return '0xa';
      if (method === 'eth_getBlockByNumber') {
        expect(params).toEqual(['0xa', false]);
        return { hash: '0xshared' };
      }
      throw new Error(`Unexpected method: ${method}`);
    });
    getBlockNumber.mockResolvedValue(11n);
    getBlock.mockImplementation(async ({ blockNumber }: { blockNumber: bigint }) => {
      expect(blockNumber).toBe(10n);
      return { hash: '0xshared' };
    });

    await expect(walletRpcMatchesAppRpc(31337)).resolves.toBe(true);
    expect(request).toHaveBeenCalledTimes(3);
  });

  it('returns false when hashes differ at the shared fixed block height', async () => {
    installEthereumRequest(async ({ method, params }) => {
      if (method === 'eth_chainId') return '0x7a69';
      if (method === 'eth_blockNumber') return '0xc';
      if (method === 'eth_getBlockByNumber') {
        expect(params).toEqual(['0xc', false]);
        return { hash: '0xwallet' };
      }
      throw new Error(`Unexpected method: ${method}`);
    });
    getBlockNumber.mockResolvedValue(12n);
    getBlock.mockResolvedValue({ hash: '0xapp' });

    await expect(walletRpcMatchesAppRpc(31337)).resolves.toBe(false);
  });

  it('returns null when the check is not applicable to the configured local chain', async () => {
    const request = installEthereumRequest(async ({ method }) => {
      throw new Error(`Unexpected method: ${method}`);
    });

    await expect(walletRpcMatchesAppRpc(1)).resolves.toBeNull();
    expect(request).not.toHaveBeenCalled();
  });

  it('returns null when either provider cannot read the shared fixed block', async () => {
    installEthereumRequest(async ({ method, params }) => {
      if (method === 'eth_chainId') return '0x7a69';
      if (method === 'eth_blockNumber') return '0xb';
      if (method === 'eth_getBlockByNumber') {
        expect(params).toEqual(['0xb', false]);
        return null;
      }
      throw new Error(`Unexpected method: ${method}`);
    });
    getBlockNumber.mockResolvedValue(11n);
    getBlock.mockResolvedValue({ hash: '0xapp' });

    await expect(walletRpcMatchesAppRpc(31337)).resolves.toBeNull();
  });
});
