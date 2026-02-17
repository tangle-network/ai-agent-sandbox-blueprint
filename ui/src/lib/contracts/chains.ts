import { defineChain } from 'viem';
import { mainnet } from 'viem/chains';
import type { Address, Chain } from 'viem';

function resolveRpcUrl(): string {
  const configured = import.meta.env.VITE_RPC_URL ?? 'http://localhost:8545';
  if (typeof window === 'undefined') return configured;
  try {
    const rpc = new URL(configured);
    const isLocalRpc = rpc.hostname === '127.0.0.1' || rpc.hostname === 'localhost';
    const pageHost = window.location.hostname;
    const isLocalPage = pageHost === '127.0.0.1' || pageHost === 'localhost';
    if (isLocalRpc && !isLocalPage) {
      rpc.hostname = pageHost;
      return rpc.toString().replace(/\/$/, '');
    }
  } catch {
    // malformed
  }
  return configured;
}

export const rpcUrl = resolveRpcUrl();

export const tangleLocal = defineChain({
  id: Number(import.meta.env.VITE_CHAIN_ID ?? 31337),
  name: 'Tangle Local',
  nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
  rpcUrls: { default: { http: [rpcUrl] } },
  blockExplorers: { default: { name: 'Explorer', url: '' } },
  contracts: { multicall3: { address: '0xcA11bde05977b3631167028862bE2a173976CA11' } },
});

export const tangleTestnet = defineChain({
  id: 3799,
  name: 'Tangle Testnet',
  nativeCurrency: { name: 'Tangle', symbol: 'tTNT', decimals: 18 },
  rpcUrls: {
    default: {
      http: ['https://testnet-rpc.tangle.tools'],
      webSocket: ['wss://testnet-rpc.tangle.tools'],
    },
  },
  blockExplorers: { default: { name: 'Tangle Explorer', url: 'https://testnet-explorer.tangle.tools' } },
  contracts: { multicall3: { address: '0xcA11bde05977b3631167028862bE2a173976CA11' } },
});

export const tangleMainnet = defineChain({
  id: 5845,
  name: 'Tangle',
  nativeCurrency: { name: 'Tangle', symbol: 'TNT', decimals: 18 },
  rpcUrls: {
    default: {
      http: ['https://rpc.tangle.tools'],
      webSocket: ['wss://rpc.tangle.tools'],
    },
  },
  blockExplorers: { default: { name: 'Tangle Explorer', url: 'https://explorer.tangle.tools' } },
  contracts: { multicall3: { address: '0xcA11bde05977b3631167028862bE2a173976CA11' } },
});

export interface NetworkConfig {
  chain: Chain;
  rpcUrl: string;
  label: string;
  shortLabel: string;
  addresses: {
    sandboxBlueprint: Address;
    jobs: Address;
    services: Address;
  };
}

export const networks: Record<number, NetworkConfig> = {
  [tangleLocal.id]: {
    chain: tangleLocal,
    rpcUrl,
    label: 'Tangle Local',
    shortLabel: 'Local',
    addresses: {
      sandboxBlueprint: (import.meta.env.VITE_SANDBOX_CONTRACT ?? '0x0000000000000000000000000000000000000000') as Address,
      jobs: (import.meta.env.VITE_JOBS_CONTRACT ?? '0x0000000000000000000000000000000000000000') as Address,
      services: (import.meta.env.VITE_SERVICES_CONTRACT ?? '0x0000000000000000000000000000000000000000') as Address,
    },
  },
  [tangleTestnet.id]: {
    chain: tangleTestnet,
    rpcUrl: 'https://testnet-rpc.tangle.tools',
    label: 'Tangle Testnet',
    shortLabel: 'Testnet',
    addresses: {
      sandboxBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      jobs: '0x0000000000000000000000000000000000000000' as Address,
      services: '0x0000000000000000000000000000000000000000' as Address,
    },
  },
  [tangleMainnet.id]: {
    chain: tangleMainnet,
    rpcUrl: 'https://rpc.tangle.tools',
    label: 'Tangle Mainnet',
    shortLabel: 'Mainnet',
    addresses: {
      sandboxBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      jobs: '0x0000000000000000000000000000000000000000' as Address,
      services: '0x0000000000000000000000000000000000000000' as Address,
    },
  },
};

export { mainnet };
export const allTangleChains = [tangleLocal, tangleTestnet, tangleMainnet] as const;
