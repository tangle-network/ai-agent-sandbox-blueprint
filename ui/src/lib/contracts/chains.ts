/**
 * Re-exports chain utilities from @tangle/blueprint-ui and configures
 * sandbox-specific network addresses at module load time.
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
} from '@tangle/blueprint-ui';

export {
  tangleLocal,
  tangleTestnet,
  tangleMainnet,
  rpcUrl,
  allTangleChains,
  mainnet,
  resolveRpcUrl,
  configureNetworks,
  getNetworks,
} from '@tangle/blueprint-ui';
export type { CoreAddresses, NetworkConfig } from '@tangle/blueprint-ui';

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
      sandboxBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      instanceBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      teeInstanceBlueprint: '0x0000000000000000000000000000000000000000' as Address,
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
      instanceBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      teeInstanceBlueprint: '0x0000000000000000000000000000000000000000' as Address,
      jobs: '0x0000000000000000000000000000000000000000' as Address,
      services: '0x0000000000000000000000000000000000000000' as Address,
    },
  },
});

/** Backwards-compatible accessor — use getNetworks<SandboxAddresses>() for typed access. */
export const networks = getNetworks<SandboxAddresses>();
