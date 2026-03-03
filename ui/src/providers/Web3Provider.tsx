import { createConfig } from 'wagmi';
import { ConnectKitProvider, getDefaultConfig } from 'connectkit';
import { type ReactNode, useEffect } from 'react';
import { useReconnect } from 'wagmi';
import {
  mainnet,
  resolveRpcUrl,
  tangleLocal,
  tangleMainnet,
  tangleTestnet,
} from '@tangle/blueprint-ui';
import { http } from 'viem';
import { Web3Shell } from '~/components/layout/Web3Shell';

const walletConnectProjectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID || '';
const tangleWalletChains = [tangleLocal, tangleTestnet, tangleMainnet, mainnet] as const;

function createTangleTransports() {
  return {
    [tangleLocal.id]: http(resolveRpcUrl(import.meta.env.VITE_RPC_URL)),
    [tangleTestnet.id]: http('https://testnet-rpc.tangle.tools'),
    [tangleMainnet.id]: http('https://rpc.tangle.tools'),
    [mainnet.id]: http(),
  };
}

const config = createConfig(
  getDefaultConfig({
    chains: tangleWalletChains as any,
    transports: createTangleTransports() as any,
    walletConnectProjectId,
    appName: 'Tangle Sandbox Cloud',
    appDescription: 'AI Agent Sandbox Provisioning on Tangle Network',
    appUrl: typeof window !== 'undefined' ? window.location.origin : 'https://cloud.tangle.tools',
    appIcon: '/favicon.svg',
  }),
);

export { config };

/**
 * Eagerly reconnect using only the injected (MetaMask) connector.
 * wagmi's default reconnect tries ALL connectors including WalletConnect,
 * which can be slow when no project ID is set or on insecure contexts.
 * This fires immediately and wins the race against WalletConnect's timeout.
 */
function FastReconnect({ children }: { children: ReactNode }) {
  const { reconnect } = useReconnect();

  useEffect(() => {
    const inj = config.connectors.find((c) => c.type === 'injected');
    if (inj) {
      reconnect({ connectors: [inj] });
    }
  }, [reconnect]);

  return <>{children}</>;
}

export function Web3Provider({ children }: { children: ReactNode }) {
  return (
    <Web3Shell config={config as any}>
      <ConnectKitProvider
        theme="auto"
        mode="auto"
        options={{
          initialChainId: undefined,
        }}
      >
        <FastReconnect>
          {children}
        </FastReconnect>
      </ConnectKitProvider>
    </Web3Shell>
  );
}
