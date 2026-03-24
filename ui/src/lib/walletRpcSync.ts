import { publicClient } from '@tangle-network/blueprint-ui';

function localChainId(): number {
  return Number(import.meta.env.VITE_CHAIN_ID ?? 31337);
}

export function expectedLocalRpcUrl(): string {
  const envRpc = (import.meta.env.VITE_RPC_URL as string | undefined) || 'http://localhost:8645';
  return envRpc.replace('127.0.0.1', 'localhost');
}

/**
 * Detect the "same chain ID, different local RPC" case by comparing a fixed
 * block height reported by both the wallet provider and the app's configured
 * public client.
 *
 * Returns:
 * - `true` when both point at the same local chain
 * - `false` when both report the local chain ID but different hashes at the
 *   same shared block height
 * - `null` when the check is not applicable or could not be completed
 */
export async function walletRpcMatchesAppRpc(expectedChainId: number): Promise<boolean | null> {
  if (typeof window === 'undefined') return null;

  const provider: any = (window as any).ethereum;
  if (!provider?.request) return null;
  if (expectedChainId !== localChainId()) return null;

  try {
    const [walletChainHex, walletBlockNumberHex, appBlockNumber] = await Promise.all([
      provider.request({ method: 'eth_chainId', params: [] }) as Promise<string>,
      provider.request({ method: 'eth_blockNumber', params: [] }) as Promise<string>,
      publicClient.getBlockNumber(),
    ]);

    const walletChainId = Number(walletChainHex);
    if (walletChainId !== expectedChainId) return null;
    const walletBlockNumber = BigInt(walletBlockNumberHex);
    const comparisonBlockNumber = walletBlockNumber < appBlockNumber
      ? walletBlockNumber
      : appBlockNumber;
    const comparisonBlockHex = `0x${comparisonBlockNumber.toString(16)}`;

    const [walletBlock, appBlock] = await Promise.all([
      provider.request({
        method: 'eth_getBlockByNumber',
        params: [comparisonBlockHex, false],
      }) as Promise<{ hash?: string } | null>,
      publicClient.getBlock({ blockNumber: comparisonBlockNumber }),
    ]);

    const walletHash = walletBlock?.hash?.toLowerCase();
    const appHash = appBlock.hash.toLowerCase();
    if (!walletHash || !appHash) return null;

    return walletHash === appHash;
  } catch {
    return null;
  }
}
