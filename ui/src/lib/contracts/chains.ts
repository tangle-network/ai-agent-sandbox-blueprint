/**
 * Sandbox-specific network configuration.
 * For chain/network primitives, import from '@tangle-network/blueprint-ui' directly.
 */
import type { Address } from 'viem';
import {
  tangleLocal,
  tangleTestnet,
  tangleMainnet,
  rpcUrl,
  configureNetworks,
  getNetworks,
  type CoreAddresses,
} from '@tangle-network/blueprint-ui';

/** Sandbox-specific addresses — extends CoreAddresses with blueprint BSM addresses. */
export interface SandboxAddresses extends CoreAddresses {
  sandboxBlueprint: Address;
  instanceBlueprint: Address;
  teeInstanceBlueprint: Address;
}

// Configure sandbox networks at module load time.
configureNetworks<SandboxAddresses>({
  [tangleLocal.id]: {
    chain: tangleLocal,
    rpcUrl,
    label: 'Tangle Local',
    shortLabel: 'Local',
    addresses: {
      sandboxBlueprint: (import.meta.env.VITE_SANDBOX_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      instanceBlueprint: (import.meta.env.VITE_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      teeInstanceBlueprint: (import.meta.env.VITE_TEE_INSTANCE_BSM ?? '0x0000000000000000000000000000000000000000') as Address,
      jobs: (import.meta.env.VITE_TANGLE_CONTRACT ?? '0x0000000000000000000000000000000000000000') as Address,
      services: (import.meta.env.VITE_TANGLE_CONTRACT ?? '0x0000000000000000000000000000000000000000') as Address,
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
});

/** Backwards-compatible accessor — use getNetworks<SandboxAddresses>() for typed access. */
export const networks = getNetworks<SandboxAddresses>();

const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000';

/** Check if a contract address is non-zero (i.e., actually deployed). */
export function isContractDeployed(address: string | undefined): boolean {
  if (!address) return false;
  return address.toLowerCase() !== ZERO_ADDRESS;
}

/** Check if the core contracts (jobs, services, blueprint BSM) are deployed for a given network.
 *  If chainId is provided, checks that specific network.
 *  Otherwise falls back to checking if ANY network has deployed contracts. */
export function areContractsDeployed(chainId?: number): boolean {
  const nets = getNetworks<SandboxAddresses>();
  if (chainId) {
    const net = Object.values(nets).find(n => n?.chain?.id === chainId);
    if (!net?.addresses) return false;
    return isContractDeployed(net.addresses.jobs) && isContractDeployed(net.addresses.services);
  }
  // Fallback: check all networks, return true if ANY has deployed contracts
  return Object.values(nets).some(net =>
    net?.addresses &&
    isContractDeployed(net.addresses.jobs) &&
    isContractDeployed(net.addresses.services),
  );
}
