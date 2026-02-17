import { createPublicClient, http } from 'viem';
import type { PublicClient } from 'viem';
import { atom } from 'nanostores';
import { networks, tangleLocal } from './chains';
import { persistedAtom } from '~/lib/stores/persistedAtom';

const defaultChainId = Number(import.meta.env.VITE_CHAIN_ID ?? tangleLocal.id);

export const selectedChainIdStore = persistedAtom<number>({
  key: 'sandbox_cloud_selected_chain',
  initial: defaultChainId,
});

const clientCache = new Map<number, PublicClient>();

function getOrCreateClient(chainId: number): PublicClient {
  const cached = clientCache.get(chainId);
  if (cached) return cached;
  const net = networks[chainId];
  if (!net) {
    const fallback = networks[defaultChainId]!;
    return createPublicClient({ chain: fallback.chain, transport: http(fallback.rpcUrl) });
  }
  const client = createPublicClient({ chain: net.chain, transport: http(net.rpcUrl) });
  clientCache.set(chainId, client);
  return client;
}

export const publicClientStore = atom<PublicClient>(getOrCreateClient(selectedChainIdStore.get()));

selectedChainIdStore.subscribe((chainId) => {
  publicClientStore.set(getOrCreateClient(chainId));
});

export function getPublicClient(): PublicClient {
  return publicClientStore.get();
}

export const publicClient = new Proxy({} as PublicClient, {
  get(_target, prop) {
    const client = getOrCreateClient(selectedChainIdStore.get());
    const value = (client as any)[prop];
    return typeof value === 'function' ? value.bind(client) : value;
  },
});

export function getAddresses() {
  const net = networks[selectedChainIdStore.get()];
  return net?.addresses ?? networks[defaultChainId]!.addresses;
}
