/**
 * Sandbox-specific network configuration.
 * For chain/network primitives, import from '@tangle-network/blueprint-ui' directly.
 */
import type { Address } from 'viem';
import { baseSepolia as viemBaseSepolia } from 'viem/chains';
import {
  tangleLocal,
  tangleTestnet,
  tangleMainnet,
  rpcUrl,
  configureNetworks,
  getNetworks,
  sanitizeSelectedChainId,
  selectedChainIdStore,
  type CoreAddresses,
} from '@tangle-network/blueprint-ui';

/** Sandbox-specific addresses — extends CoreAddresses with blueprint BSM addresses. */
export interface SandboxAddresses extends CoreAddresses {
  sandboxBlueprint: Address;
  instanceBlueprint: Address;
  teeInstanceBlueprint: Address;
}

const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000';
const BASE_SEPOLIA_RPC_URL = import.meta.env.VITE_BASE_SEPOLIA_RPC_URL ?? 'https://sepolia.base.org';
const BASE_SEPOLIA_TANGLE = '0x8299d60f373f3a4a8c4878e335cb9d840e6e3730';
const BASE_SEPOLIA_SANDBOX_BSM = '0x281d2d1160d80070ebe8989a529b6732c8403625';
const BASE_SEPOLIA_INSTANCE_BSM = '0xde25dad1757e5dab5230d44779d7de6ad8181c5c';
const BASE_SEPOLIA_TEE_INSTANCE_BSM = '0x6d6debfa88260558597ad912439ea1949962b3eb';

export const baseSepolia = {
  ...viemBaseSepolia,
  rpcUrls: {
    ...viemBaseSepolia.rpcUrls,
    default: { http: [BASE_SEPOLIA_RPC_URL] },
    public: { http: [BASE_SEPOLIA_RPC_URL] },
  },
};

const enableLocalNetwork = import.meta.env.VITE_ENABLE_LOCAL_NETWORK === 'true';
const localNetworkConfig = enableLocalNetwork
  ? {
      [tangleLocal.id]: {
        chain: tangleLocal,
        rpcUrl,
        label: 'Tangle Local',
        shortLabel: 'Local',
        addresses: {
          sandboxBlueprint: (import.meta.env.VITE_SANDBOX_BSM ?? ZERO_ADDRESS) as Address,
          instanceBlueprint: (import.meta.env.VITE_INSTANCE_BSM ?? ZERO_ADDRESS) as Address,
          teeInstanceBlueprint: (import.meta.env.VITE_TEE_INSTANCE_BSM ?? ZERO_ADDRESS) as Address,
          jobs: (import.meta.env.VITE_TANGLE_CONTRACT ?? ZERO_ADDRESS) as Address,
          services: (import.meta.env.VITE_TANGLE_CONTRACT ?? ZERO_ADDRESS) as Address,
        },
      },
    }
  : {};

// Configure sandbox networks at module load time.
configureNetworks<SandboxAddresses>({
  [baseSepolia.id]: {
    chain: baseSepolia,
    rpcUrl: BASE_SEPOLIA_RPC_URL,
    label: 'Base Sepolia',
    shortLabel: 'Base',
    addresses: {
      sandboxBlueprint: (import.meta.env.VITE_BASE_SEPOLIA_SANDBOX_BSM ?? BASE_SEPOLIA_SANDBOX_BSM) as Address,
      instanceBlueprint: (import.meta.env.VITE_BASE_SEPOLIA_INSTANCE_BSM ?? BASE_SEPOLIA_INSTANCE_BSM) as Address,
      teeInstanceBlueprint: (import.meta.env.VITE_BASE_SEPOLIA_TEE_INSTANCE_BSM ?? BASE_SEPOLIA_TEE_INSTANCE_BSM) as Address,
      jobs: (import.meta.env.VITE_BASE_SEPOLIA_TANGLE_CONTRACT ?? BASE_SEPOLIA_TANGLE) as Address,
      services: (import.meta.env.VITE_BASE_SEPOLIA_TANGLE_CONTRACT ?? BASE_SEPOLIA_TANGLE) as Address,
    },
  },
  [tangleTestnet.id]: {
    chain: tangleTestnet,
    rpcUrl: 'https://testnet-rpc.tangle.tools',
    label: 'Tangle Testnet',
    shortLabel: 'Testnet',
    addresses: {
      sandboxBlueprint: (import.meta.env.VITE_TESTNET_SANDBOX_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      instanceBlueprint: (import.meta.env.VITE_TESTNET_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      teeInstanceBlueprint: (import.meta.env.VITE_TESTNET_TEE_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      jobs: (import.meta.env.VITE_TESTNET_JOBS_ADDRESS ?? '0x0000000000000000000000000000000000000000') as Address,
      services: (import.meta.env.VITE_TESTNET_SERVICES_ADDRESS ?? '0x0000000000000000000000000000000000000000') as Address,
    },
  },
  [tangleMainnet.id]: {
    chain: tangleMainnet,
    rpcUrl: 'https://rpc.tangle.tools',
    label: 'Tangle Mainnet',
    shortLabel: 'Mainnet',
    addresses: {
      sandboxBlueprint: (import.meta.env.VITE_MAINNET_SANDBOX_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      instanceBlueprint: (import.meta.env.VITE_MAINNET_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      teeInstanceBlueprint: (import.meta.env.VITE_MAINNET_TEE_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      jobs: (import.meta.env.VITE_MAINNET_JOBS_ADDRESS ?? '0x0000000000000000000000000000000000000000') as Address,
      services: (import.meta.env.VITE_MAINNET_SERVICES_ADDRESS ?? '0x0000000000000000000000000000000000000000') as Address,
    },
  },
  ...localNetworkConfig,
});

if (!enableLocalNetwork && selectedChainIdStore.get() === tangleLocal.id) {
  selectedChainIdStore.set(baseSepolia.id);
}

sanitizeSelectedChainId();

/** Backwards-compatible accessor — use getNetworks<SandboxAddresses>() for typed access. */
export const networks = getNetworks<SandboxAddresses>();

/** Check if a contract address is non-zero (i.e., actually deployed). */
export function isContractDeployed(address: string | undefined): boolean {
  if (!address) return false;
  return address.toLowerCase() !== ZERO_ADDRESS;
}
